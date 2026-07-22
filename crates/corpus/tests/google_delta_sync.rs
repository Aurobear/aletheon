use async_trait::async_trait;
use corpus::tools::google::{
    CalendarSyncConfig, CalendarSynchronizer, DriveSyncConfig, DriveSyncHealthEvent,
    DriveSynchronizer, GoogleAccessToken, GoogleApiClient, GoogleApiEndpoints, GoogleApiError,
    GoogleCalendarAdapter, GoogleCredentialSource, GoogleDriveAdapter,
};
use fabric::{ExternalCapabilityId, ExternalEvent, ExternalIdentityId, PrincipalId};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct Credentials {
    owner: PrincipalId,
    account: ExternalIdentityId,
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
        if ![
            ExternalCapabilityId::new("calendar.read").unwrap(),
            ExternalCapabilityId::new("file.read").unwrap(),
        ]
        .contains(&scope)
        {
            return Err(GoogleApiError::ScopeDenied);
        }
        Ok(())
    }
}

struct MockResponse {
    status: StatusCode,
    body: Vec<u8>,
    content_type: &'static str,
}

impl MockResponse {
    fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.into(),
            content_type: "application/json",
        }
    }

    fn status(status: StatusCode) -> Self {
        Self {
            status,
            body: Vec::new(),
            content_type: "application/json",
        }
    }

    fn bytes(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.into(),
            content_type: "application/octet-stream",
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
        while let Ok((stream, _)) = listener.accept().await {
            let queue = queue.clone();
            let captured = captured.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let queue = queue.clone();
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().push(request.uri().to_string());
                        let response = queue.lock().unwrap().pop_front().unwrap();
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(response.status)
                                .header("content-type", response.content_type)
                                .body(Full::new(Bytes::from(response.body)))
                                .unwrap(),
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

fn client(endpoint: &str) -> (GoogleApiClient, PrincipalId, ExternalIdentityId) {
    let principal = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let credentials = Arc::new(Credentials {
        owner: principal.clone(),
        account,
    });
    let client = GoogleApiClient::new_local(
        credentials,
        GoogleApiEndpoints {
            gmail_base: endpoint.into(),
            calendar_base: endpoint.into(),
            drive_base: endpoint.into(),
        },
    )
    .unwrap();
    (client, principal, account)
}

fn calendar_config() -> CalendarSyncConfig {
    CalendarSyncConfig {
        window_start_ms: 1_700_000_000_000,
        window_end_ms: 1_800_000_000_000,
        timezone: "UTC".into(),
        max_pages: 5,
        page_size: 2,
    }
}

fn active_event(id: &str, etag: &str) -> String {
    format!(
        r#"{{"id":"{id}","etag":"{etag}","updated":"2026-07-14T10:00:00Z","status":"confirmed","summary":"event","start":{{"dateTime":"2026-07-15T10:00:00Z","timeZone":"UTC"}},"end":{{"dateTime":"2026-07-15T11:00:00Z","timeZone":"UTC"}}}}"#
    )
}

#[tokio::test]
async fn calendar_initial_window_paginates_recurring_instances_and_continues_with_tombstones() {
    let first = format!(
        r#"{{"items":[{}],"nextPageToken":"page-2"}}"#,
        active_event("series_instance_1", "v1")
    );
    let second = format!(
        r#"{{"items":[{}],"nextSyncToken":"sync-1"}}"#,
        active_event("series_instance_2", "v1")
    );
    let incremental = br#"{"items":[{"id":"series_instance_1","etag":"v2","updated":"2026-07-14T11:00:00Z","status":"cancelled"}],"nextSyncToken":"sync-2"}"#.to_vec();
    let (endpoint, requests) = server(vec![
        MockResponse::json(first),
        MockResponse::json(second),
        MockResponse::json(incremental),
    ])
    .await;
    let (client, principal, account) = client(&endpoint);
    let sync =
        CalendarSynchronizer::new(GoogleCalendarAdapter::new(client), calendar_config()).unwrap();
    let initial = sync
        .synchronize(
            &principal,
            account,
            None,
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(initial.successor_cursor, "sync-1");
    assert_eq!(initial.events.len(), 2);
    assert!(initial
        .events
        .iter()
        .all(|event| matches!(event.event, ExternalEvent::CalendarEventCreated(_))));

    let delta = sync
        .synchronize(
            &principal,
            account,
            Some("sync-1"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(delta.successor_cursor, "sync-2");
    assert!(matches!(
        delta.events[0].event,
        ExternalEvent::CalendarEventDeleted(_)
    ));
    let requests = requests.lock().unwrap();
    assert!(requests[0].contains("timeMin=") && requests[0].contains("showDeleted=true"));
    assert!(requests[1].contains("pageToken=page-2"));
    assert!(requests[2].contains("syncToken=sync-1") && !requests[2].contains("timeMin="));
}

#[tokio::test]
async fn calendar_410_rebuilds_only_window_and_deduplicates_before_replacing_token() {
    let rebuild = format!(
        r#"{{"items":[{},{}],"nextSyncToken":"sync-new"}}"#,
        active_event("same", "v1"),
        active_event("same", "v1")
    );
    let (endpoint, requests) = server(vec![
        MockResponse::status(StatusCode::GONE),
        MockResponse::json(rebuild),
    ])
    .await;
    let (client, principal, account) = client(&endpoint);
    let sync =
        CalendarSynchronizer::new(GoogleCalendarAdapter::new(client), calendar_config()).unwrap();
    let batch = sync
        .synchronize(
            &principal,
            account,
            Some("expired"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(batch.reconciled);
    assert_eq!(batch.input_cursor.as_deref(), Some("expired"));
    assert_eq!(batch.successor_cursor, "sync-new");
    assert_eq!(batch.events.len(), 1);
    let requests = requests.lock().unwrap();
    assert!(requests[0].contains("syncToken=expired"));
    assert!(requests[1].contains("timeMin=") && requests[1].contains("timeMax="));
}

fn drive_config() -> DriveSyncConfig {
    DriveSyncConfig {
        selected_file_ids: HashSet::from([
            "selected".into(),
            "deleted".into(),
            "large".into(),
            "mime".into(),
        ]),
        content_mime_allowlist: HashSet::from(["text/plain".into()]),
        download_content: true,
        max_content_bytes: 16,
        max_pages: 5,
        max_changes: 10,
        page_size: 3,
    }
}

#[tokio::test]
async fn drive_baseline_and_changes_enforce_selection_shared_drive_and_content_policy() {
    let page_one = br#"{"changes":[{"fileId":"ignored","time":"2026-07-14T10:00:00Z","file":{"id":"ignored","name":"ignored","mimeType":"text/plain","size":"4","modifiedTime":"2026-07-14T10:00:00Z","version":"1"}},{"fileId":"selected","time":"2026-07-14T10:00:00Z","file":{"id":"selected","name":"selected.txt","mimeType":"text/plain","size":"4","modifiedTime":"2026-07-14T10:00:00Z","version":"1"}}],"nextPageToken":"page-2"}"#.to_vec();
    let page_two = br#"{"changes":[{"fileId":"deleted","removed":true,"time":"2026-07-14T11:00:00Z"},{"fileId":"large","time":"2026-07-14T12:00:00Z","file":{"id":"large","name":"large.txt","mimeType":"text/plain","size":"100","modifiedTime":"2026-07-14T12:00:00Z","version":"2"}},{"fileId":"mime","time":"2026-07-14T13:00:00Z","file":{"id":"mime","name":"image.png","mimeType":"image/png","size":"4","modifiedTime":"2026-07-14T13:00:00Z","version":"3"}}],"newStartPageToken":"cursor-2"}"#.to_vec();
    let (endpoint, requests) = server(vec![
        MockResponse::json(br#"{"startPageToken":"cursor-1"}"#.to_vec()),
        MockResponse::json(page_one),
        MockResponse::bytes(b"data".to_vec()),
        MockResponse::json(page_two),
    ])
    .await;
    let (client, principal, account) = client(&endpoint);
    let sync = DriveSynchronizer::new(GoogleDriveAdapter::new(client), drive_config()).unwrap();
    let baseline = sync
        .synchronize(
            &principal,
            account,
            None,
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(baseline.baseline_only);
    assert_eq!(baseline.successor_cursor, "cursor-1");

    let batch = sync
        .synchronize(
            &principal,
            account,
            Some("cursor-1"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(batch.successor_cursor, "cursor-2");
    assert_eq!(batch.events.len(), 4);
    assert_eq!(batch.artifacts.len(), 1);
    assert_eq!(batch.artifacts[0].bytes, b"data");
    assert!(matches!(
        batch.events[1].event,
        ExternalEvent::FileDeleted(_)
    ));
    match &batch.events[2].event {
        ExternalEvent::FileUpdated(file) => assert!(file.content.is_none()),
        other => panic!("unexpected event: {other:?}"),
    }
    match &batch.events[3].event {
        ExternalEvent::FileUpdated(file) => assert!(file.content.is_none()),
        other => panic!("unexpected event: {other:?}"),
    }
    let requests = requests.lock().unwrap();
    assert!(requests[0].contains("supportsAllDrives=true"));
    assert!(requests[1].contains("includeItemsFromAllDrives=true"));
    assert!(requests[1].contains("supportsAllDrives=true"));
    assert!(requests[2].contains("/files/selected?alt=media"));
    assert!(requests[3].contains("pageToken=page-2"));
}

#[tokio::test]
async fn drive_expired_cursor_resets_bounded_baseline_without_scanning_files() {
    let (endpoint, requests) = server(vec![
        MockResponse::status(StatusCode::GONE),
        MockResponse::json(br#"{"startPageToken":"fresh"}"#.to_vec()),
    ])
    .await;
    let (client, principal, account) = client(&endpoint);
    let sync = DriveSynchronizer::new(GoogleDriveAdapter::new(client), drive_config()).unwrap();
    let batch = sync
        .synchronize(
            &principal,
            account,
            Some("expired"),
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(batch.reconciled && batch.baseline_only);
    assert_eq!(batch.successor_cursor, "fresh");
    assert_eq!(
        batch.health_events,
        vec![DriveSyncHealthEvent::CursorExpiredBaselineReset]
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("/changes?"));
    assert!(requests[1].starts_with("/changes/startPageToken?"));
}
