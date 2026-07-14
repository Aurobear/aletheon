use async_trait::async_trait;
use corpus::tools::google::{
    CalendarCapability, GmailCapability, GoogleAccessToken, GoogleApiClient, GoogleApiEndpoints,
    GoogleApiError, GoogleCalendarAdapter, GoogleCredentialSource, GoogleGmailAdapter,
};
use fabric::{CalendarTimeRange, ExternalIdentityId, ExternalScope, GmailQuery, PrincipalId};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct Credentials {
    owner: PrincipalId,
    account: ExternalIdentityId,
    allowed_scope: ExternalScope,
    access_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
}

impl Credentials {
    fn new(owner: PrincipalId, account: ExternalIdentityId, allowed_scope: ExternalScope) -> Self {
        Self {
            owner,
            account,
            allowed_scope,
            access_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
        }
    }

    fn authorize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
    ) -> Result<(), GoogleApiError> {
        if principal != &self.owner || account != self.account {
            return Err(GoogleApiError::UnauthorizedAccount);
        }
        if required_scope != self.allowed_scope || required_scope.is_write() {
            return Err(GoogleApiError::ScopeDenied);
        }
        Ok(())
    }
}

#[async_trait]
impl GoogleCredentialSource for Credentials {
    async fn access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, required_scope)?;
        self.access_calls.fetch_add(1, Ordering::SeqCst);
        GoogleAccessToken::new("access-secret".into())
    }

    async fn refresh_access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, required_scope)?;
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        GoogleAccessToken::new("refreshed-secret".into())
    }
}

struct MockResponse {
    status: StatusCode,
    body: Vec<u8>,
    retry_after: Option<&'static str>,
}

impl MockResponse {
    fn json(status: StatusCode, body: &'static str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            retry_after: None,
        }
    }
}

async fn server(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let queue = responses.clone();
    let captured = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let queue = queue.clone();
            let captured = captured.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let queue = queue.clone();
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().push(request.uri().to_string());
                        let response = queue.lock().unwrap().pop_front().unwrap();
                        let mut builder = Response::builder()
                            .status(response.status)
                            .header("content-type", "application/json");
                        if let Some(retry_after) = response.retry_after {
                            builder = builder.header("retry-after", retry_after);
                        }
                        Ok::<_, hyper::Error>(
                            builder.body(Full::new(Bytes::from(response.body))).unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (format!("http://{address}"), requests)
}

fn gmail(endpoint: &str, credentials: Arc<Credentials>) -> GoogleGmailAdapter {
    GoogleGmailAdapter::new(
        GoogleApiClient::new(
            credentials,
            GoogleApiEndpoints {
                gmail_base: endpoint.into(),
                calendar_base: endpoint.into(),
                drive_base: endpoint.into(),
            },
        )
        .unwrap(),
    )
}

fn calendar(endpoint: &str, credentials: Arc<Credentials>) -> GoogleCalendarAdapter {
    GoogleCalendarAdapter::new(
        GoogleApiClient::new(
            credentials,
            GoogleApiEndpoints {
                gmail_base: endpoint.into(),
                calendar_base: endpoint.into(),
                drive_base: endpoint.into(),
            },
        )
        .unwrap(),
    )
}

fn metadata(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","threadId":"thread-{id}","labelIds":["UNREAD","IMPORTANT"],"snippet":"bounded","internalDate":"1000","historyId":"7","payload":{{"headers":[{{"name":"Subject","value":"subject"}},{{"name":"From","value":"sender@example.com"}}],"mimeType":"text/plain","body":{{"data":"aGVsbG8"}}}}}}"#
    )
}

#[tokio::test]
async fn gmail_pagination_metadata_and_explicit_read_are_bounded() {
    let meta = metadata("m1");
    let (endpoint, requests) = server(vec![
        MockResponse::json(
            StatusCode::OK,
            r#"{"messages":[{"id":"m1"}],"nextPageToken":"next-page"}"#,
        ),
        MockResponse {
            status: StatusCode::OK,
            body: meta.clone().into_bytes(),
            retry_after: None,
        },
        MockResponse {
            status: StatusCode::OK,
            body: meta.into_bytes(),
            retry_after: None,
        },
    ])
    .await;
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::GmailReadonly,
    ));
    let adapter = gmail(&endpoint, credentials);
    let page = adapter
        .search_messages(
            &owner,
            GmailQuery {
                account_id: account,
                query: "is:unread".into(),
                page_size: 10,
                page_token: Some("input-page".into()),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(page.next_page_token.as_deref(), Some("next-page"));
    assert_eq!(page.messages.len(), 1);
    assert!(page.messages[0].important);
    let message = adapter
        .read_message(&owner, account, "m1", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(message.body_text, "hello");
    let paths = requests.lock().unwrap();
    assert!(paths[0].contains("pageToken=input-page"));
    assert!(paths[1].contains("format=metadata"));
    assert!(paths[2].contains("format=full"));
    let snapshot = serde_json::to_string(&(page, message)).unwrap();
    assert!(!snapshot.contains("access-secret"));
    assert!(!snapshot.contains("Authorization"));
}

#[tokio::test]
async fn unauthorized_account_and_wrong_scope_never_reach_provider() {
    let (endpoint, requests) = server(vec![]).await;
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::GmailReadonly,
    ));
    let adapter = gmail(&endpoint, credentials);
    let forged = adapter
        .important_unread(
            &PrincipalId("attacker".into()),
            account,
            10,
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(forged, Err(GoogleApiError::UnauthorizedAccount));
    assert!(requests.lock().unwrap().is_empty());

    let wrong_grant = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    let denied = gmail(&endpoint, wrong_grant)
        .important_unread(&owner, account, 10, &CancellationToken::new())
        .await;
    assert_eq!(denied, Err(GoogleApiError::ScopeDenied));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unauthorized_refreshes_once_and_calendar_pagination_is_normalized() {
    let (endpoint, requests) = server(vec![
        MockResponse::json(StatusCode::UNAUTHORIZED, r#"{"secret":"provider-secret"}"#),
        MockResponse::json(
            StatusCode::OK,
            r#"{"items":[{"id":"event-1","etag":"etag-1","updated":"2026-07-15T01:00:00Z","summary":"meeting","start":{"dateTime":"2026-07-15T02:00:00Z","timeZone":"Asia/Shanghai"},"end":{"dateTime":"2026-07-15T03:00:00Z","timeZone":"Asia/Shanghai"}}],"nextPageToken":"calendar-next"}"#,
        ),
    ])
    .await;
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    let adapter = calendar(&endpoint, credentials.clone());
    let page = adapter
        .list_events(
            &owner,
            CalendarTimeRange {
                account_id: account,
                start_ms: 1_752_537_600_000,
                end_ms: 1_752_624_000_000,
                timezone: "Asia/Shanghai".into(),
                page_size: 10,
                page_token: Some("calendar-input".into()),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(credentials.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.next_page_token.as_deref(), Some("calendar-next"));
    assert!(requests.lock().unwrap()[0].contains("pageToken=calendar-input"));
}

#[tokio::test]
async fn scope_denial_rate_limit_and_malformed_payload_use_stable_errors() {
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();

    let (endpoint, _) = server(vec![MockResponse::json(
        StatusCode::FORBIDDEN,
        r#"{"token":"provider-secret"}"#,
    )])
    .await;
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    let range = CalendarTimeRange {
        account_id: account,
        start_ms: 1_000,
        end_ms: 2_000,
        timezone: "UTC".into(),
        page_size: 10,
        page_token: None,
    };
    assert_eq!(
        calendar(&endpoint, credentials)
            .list_events(&owner, range.clone(), &CancellationToken::new())
            .await,
        Err(GoogleApiError::ScopeDenied)
    );

    let (endpoint, _) = server(vec![
        MockResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: b"provider-secret".to_vec(),
            retry_after: Some("0"),
        },
        MockResponse::json(StatusCode::OK, r#"{"items":[]}"#),
    ])
    .await;
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    assert!(calendar(&endpoint, credentials)
        .list_events(&owner, range.clone(), &CancellationToken::new())
        .await
        .is_ok());

    let (endpoint, _) = server(vec![MockResponse::json(StatusCode::OK, "not-json")]).await;
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    let error = calendar(&endpoint, credentials)
        .list_events(&owner, range, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error, GoogleApiError::MalformedResponse);
    assert!(!error.to_string().contains("provider-secret"));
}

#[tokio::test]
async fn cancellation_and_response_size_limits_fail_closed() {
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let range = CalendarTimeRange {
        account_id: account,
        start_ms: 1_000,
        end_ms: 2_000,
        timezone: "UTC".into(),
        page_size: 10,
        page_token: None,
    };
    let (endpoint, _) = server(vec![]).await;
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        calendar(&endpoint, credentials)
            .list_events(&owner, range.clone(), &cancel)
            .await,
        Err(GoogleApiError::Cancelled)
    );

    let oversized = vec![b'x'; corpus::tools::google::client::MAX_GOOGLE_RESPONSE_BYTES + 1];
    let (endpoint, _) = server(vec![MockResponse {
        status: StatusCode::OK,
        body: oversized,
        retry_after: None,
    }])
    .await;
    let credentials = Arc::new(Credentials::new(
        owner.clone(),
        account,
        ExternalScope::CalendarReadonly,
    ));
    assert_eq!(
        calendar(&endpoint, credentials)
            .list_events(&owner, range, &CancellationToken::new())
            .await,
        Err(GoogleApiError::ResponseTooLarge)
    );
}
