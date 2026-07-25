use async_trait::async_trait;
use corpus::tools::google::{
    GmailHistorySyncConfig, GmailHistorySynchronizer, GmailSyncHealthEvent, GoogleAccessToken,
    GoogleApiClient, GoogleApiEndpoints, GoogleApiError, GoogleCredentialSource,
    GoogleGmailAdapter,
};
use fabric::{ExternalCapabilityId, ExternalEvent, ExternalIdentityId, PrincipalId};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

struct Credentials {
    owner: PrincipalId,
    account: ExternalIdentityId,
    refreshes: AtomicUsize,
}

#[async_trait]
impl GoogleCredentialSource for Credentials {
    async fn access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        scope: ExternalCapabilityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, scope)?;
        GoogleAccessToken::new("access-secret".into())
    }

    async fn refresh_access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        scope: ExternalCapabilityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, scope)?;
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        GoogleAccessToken::new("refreshed-secret".into())
    }
}

impl Credentials {
    fn authorize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        scope: ExternalCapabilityId,
    ) -> Result<(), GoogleApiError> {
        if principal != &self.owner || account != self.account {
            return Err(GoogleApiError::UnauthorizedAccount);
        }
        if scope != ExternalCapabilityId::new("mail.read").unwrap() {
            return Err(GoogleApiError::ScopeDenied);
        }
        Ok(())
    }
}

struct MockResponse {
    status: StatusCode,
    body: Vec<u8>,
    retry_after: Option<&'static str>,
    delay: Duration,
}

impl MockResponse {
    fn json(status: StatusCode, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            retry_after: None,
            delay: Duration::ZERO,
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
                        tokio::time::sleep(response.delay).await;
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

fn fixture(
    endpoint: &str,
) -> (
    GmailHistorySynchronizer,
    PrincipalId,
    ExternalIdentityId,
    Arc<Credentials>,
) {
    fixture_with_config(endpoint, GmailHistorySyncConfig::default())
}

fn fixture_with_config(
    endpoint: &str,
    config: GmailHistorySyncConfig,
) -> (
    GmailHistorySynchronizer,
    PrincipalId,
    ExternalIdentityId,
    Arc<Credentials>,
) {
    let principal = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let credentials = Arc::new(Credentials {
        owner: principal.clone(),
        account,
        refreshes: AtomicUsize::new(0),
    });
    let client = GoogleApiClient::new_local(
        credentials.clone(),
        GoogleApiEndpoints {
            gmail_base: endpoint.into(),
            calendar_base: endpoint.into(),
            drive_base: endpoint.into(),
        },
    )
    .unwrap();
    (
        GmailHistorySynchronizer::new(GoogleGmailAdapter::new(client), config).unwrap(),
        principal,
        account,
        credentials,
    )
}

fn metadata(id: &str, history_id: &str) -> String {
    format!(
        r#"{{"id":"{id}","threadId":"thread-{id}","labelIds":["UNREAD","IMPORTANT"],"snippet":"bounded","internalDate":"1000","historyId":"{history_id}","payload":{{"headers":[{{"name":"Subject","value":"subject"}},{{"name":"From","value":"sender@example.com"}}]}}}}"#
    )
}

#[tokio::test]
async fn initial_enrollment_records_baseline_without_replaying_mailbox() {
    let (endpoint, requests) = server(vec![MockResponse::json(
        StatusCode::OK,
        br#"{"emailAddress":"owner@example.com","historyId":"100"}"#.to_vec(),
    )])
    .await;
    let (sync, principal, account, _) = fixture(&endpoint);
    let batch = sync
        .synchronize(
            &principal,
            account,
            None,
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(batch.baseline_only);
    assert_eq!(batch.successor_cursor, "100");
    assert!(batch.events.is_empty());
    assert_eq!(requests.lock().unwrap().as_slice(), &["/users/me/profile"]);
}

#[tokio::test]
async fn paginated_add_update_delete_and_duplicate_history_are_ordered_and_bounded() {
    let (endpoint, requests) = server(vec![
        MockResponse::json(
            StatusCode::OK,
            br#"{"history":[{"id":"101","messagesAdded":[{"message":{"id":"m1"}}],"labelsAdded":[{"message":{"id":"m2"}}]}],"nextPageToken":"p2","historyId":"102"}"#.to_vec(),
        ),
        MockResponse::json(StatusCode::OK, metadata("m1", "101")),
        MockResponse::json(StatusCode::OK, metadata("m2", "101")),
        MockResponse::json(
            StatusCode::OK,
            br#"{"history":[{"id":"101","messagesAdded":[{"message":{"id":"m1"}}]},{"id":"103","messagesDeleted":[{"message":{"id":"m3"}}]}],"historyId":"103"}"#.to_vec(),
        ),
        MockResponse::json(StatusCode::OK, metadata("m1", "101")),
    ])
    .await;
    let (sync, principal, account, _) = fixture(&endpoint);
    let batch = sync
        .synchronize(
            &principal,
            account,
            Some("100"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(batch.successor_cursor, "103");
    assert_eq!(batch.events.len(), 3);
    assert!(matches!(
        batch.events[0].event,
        ExternalEvent::MailReceived(_)
    ));
    assert!(matches!(
        batch.events[1].event,
        ExternalEvent::MailUpdated(_)
    ));
    assert!(matches!(
        batch.events[2].event,
        ExternalEvent::MailDeleted(_)
    ));
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|uri| uri.contains("pageToken=p2")));
}

#[tokio::test]
async fn expired_history_runs_bounded_reconciliation_and_establishes_new_baseline() {
    let (endpoint, _) = server(vec![
        MockResponse::json(StatusCode::NOT_FOUND, b"{}".to_vec()),
        MockResponse::json(
            StatusCode::OK,
            br#"{"messages":[{"id":"m1"}],"nextPageToken":"still-more"}"#.to_vec(),
        ),
        MockResponse::json(StatusCode::OK, metadata("m1", "199")),
        MockResponse::json(
            StatusCode::OK,
            br#"{"emailAddress":"owner@example.com","historyId":"200"}"#.to_vec(),
        ),
    ])
    .await;
    let config = GmailHistorySyncConfig {
        max_pages: 1,
        max_messages: 10,
        ..Default::default()
    };
    let (sync, principal, account, _) = fixture_with_config(&endpoint, config);
    let batch = sync
        .synchronize(
            &principal,
            account,
            Some("100"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(batch.reconciled);
    assert_eq!(batch.successor_cursor, "200");
    assert_eq!(batch.events.len(), 1);
    assert_eq!(
        batch.health_events,
        vec![GmailSyncHealthEvent::ReconciliationBounded {
            pages_examined: 1,
            messages_examined: 1
        }]
    );
}

#[tokio::test]
async fn history_sync_uses_refresh_once_and_honors_rate_limit_retry() {
    let (endpoint, _) = server(vec![
        MockResponse::json(StatusCode::UNAUTHORIZED, b"{}".to_vec()),
        MockResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: b"{}".to_vec(),
            retry_after: Some("0"),
            delay: Duration::ZERO,
        },
        MockResponse::json(
            StatusCode::OK,
            br#"{"emailAddress":"owner@example.com","historyId":"100"}"#.to_vec(),
        ),
    ])
    .await;
    let (sync, principal, account, credentials) = fixture(&endpoint);
    let batch = sync
        .synchronize(
            &principal,
            account,
            None,
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(batch.successor_cursor, "100");
    assert_eq!(credentials.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_and_per_run_message_bound_fail_closed() {
    let (endpoint, _) = server(vec![MockResponse {
        status: StatusCode::OK,
        body: br#"{"emailAddress":"owner@example.com","historyId":"100"}"#.to_vec(),
        retry_after: None,
        delay: Duration::from_secs(2),
    }])
    .await;
    let (sync, principal, account, _) = fixture(&endpoint);
    let cancel = CancellationToken::new();
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        child.cancel();
    });
    assert_eq!(
        sync.synchronize(&principal, account, None, &HashSet::new(), &cancel)
            .await
            .unwrap_err()
            .to_string(),
        "google_cancelled"
    );

    let (endpoint, _) = server(vec![MockResponse::json(
        StatusCode::OK,
        br#"{"history":[{"id":"101","messagesDeleted":[{"message":{"id":"m1"}},{"message":{"id":"m2"}}]}],"historyId":"101"}"#.to_vec(),
    )])
    .await;
    let config = GmailHistorySyncConfig {
        max_messages: 1,
        ..Default::default()
    };
    let (sync, principal, account, _) = fixture_with_config(&endpoint, config);
    assert_eq!(
        sync.synchronize(
            &principal,
            account,
            Some("100"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err()
        .to_string(),
        "google_response_too_large"
    );
}
