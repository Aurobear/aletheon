use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleBinding;
use executive::r#impl::external::ExternalIdentityRepository;
use executive::r#impl::google::{
    GoogleEventDispatcher, GoogleEventSink, GooglePollBatch, GooglePollFailure, GoogleSyncManager,
    GoogleSyncManagerConfig, GoogleSyncPoller, GoogleSyncRegistration, GoogleSyncStore,
    ProjectionWrite, SyncCommit, SyncStream,
};
use fabric::{
    ExternalEventDraft, ExternalEventEnvelope, ExternalEventId, ExternalIdentityId,
    ExternalObjectRef, ExternalScope, GmailMessageSummary, GoogleEvent, IdentityProvider,
    MailChange, PrincipalId, ProviderRecordRef,
};
use kernel::chronos::TestClock;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    account: ExternalIdentityId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objectives.db");
        let account = ExternalIdentityId::new();
        ExternalIdentityRepository::open(&path)
            .unwrap()
            .bind_google(
                &PrincipalId("owner".into()),
                GoogleBinding {
                    identity_id: account,
                    provider_subject: "subject".into(),
                    email: "owner@example.com".into(),
                    scopes: vec![ExternalScope::GmailReadonly],
                },
                Some("work".into()),
                1,
            )
            .unwrap();
        Self {
            _dir: dir,
            path,
            account,
        }
    }

    fn store(&self) -> GoogleSyncStore {
        GoogleSyncStore::open(&self.path).unwrap()
    }

    fn event(&self, version: &str, source_ms: i64) -> ExternalEventEnvelope {
        let object = ExternalObjectRef {
            provider: IdentityProvider::Google,
            account_id: self.account,
            object_id: "message-1".into(),
            object_version: version.into(),
        };
        let provenance = ProviderRecordRef {
            account_id: self.account,
            provider_object_id: "message-1".into(),
            fetched_at_ms: source_ms + 1,
            source_timestamp_ms: source_ms,
            etag_or_history: Some(version.into()),
        };
        ExternalEventEnvelope::from_draft(ExternalEventDraft {
            provider: IdentityProvider::Google,
            account_id: self.account,
            provider_event_id: Some(format!("history-{version}")),
            object,
            observed_at_ms: source_ms + 1,
            source_timestamp_ms: source_ms,
            provenance: provenance.clone(),
            event: GoogleEvent::MailUpdated(MailChange {
                message: GmailMessageSummary {
                    source: provenance,
                    thread_id: "thread".into(),
                    subject: "subject".into(),
                    from: "sender@example.com".into(),
                    snippet: "snippet".into(),
                    unread: true,
                    important: true,
                },
                content: None,
            }),
        })
        .unwrap()
    }
}

#[test]
fn crash_boundaries_preserve_cursor_and_lease_expiry_allows_takeover() {
    let fixture = Fixture::new();
    let store_a = fixture.store();
    store_a
        .initialize_cursor(fixture.account, SyncStream::GmailHistory, Some("h1"), 1)
        .unwrap();
    assert!(store_a
        .acquire_lease(fixture.account, SyncStream::GmailHistory, "a", 1_000, 1_000)
        .unwrap());
    assert!(!fixture
        .store()
        .acquire_lease(fixture.account, SyncStream::GmailHistory, "b", 1_500, 1_000)
        .unwrap());
    drop(store_a); // crash before cursor commit: lease remains until expiry.
    let store_b = fixture.store();
    assert!(store_b
        .acquire_lease(fixture.account, SyncStream::GmailHistory, "b", 2_000, 1_000)
        .unwrap());
    let cursor = store_b
        .cursor(fixture.account, SyncStream::GmailHistory)
        .unwrap()
        .unwrap();
    store_b
        .commit(SyncCommit {
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            expected_cursor_token: cursor.token,
            expected_cursor_version: cursor.version,
            successor_cursor_token: "h2".into(),
            cursor_generation: 1,
            events: vec![(
                fixture.event("v1", 2_000),
                ProjectionWrite {
                    json: serde_json::json!({"subject":"subject"}),
                    tombstone: false,
                },
            )],
            committed_at_ms: 2_000,
        })
        .unwrap();
    drop(store_b); // crash after atomic cursor/outbox commit.
    let recovered = fixture.store();
    assert_eq!(
        recovered
            .cursor(fixture.account, SyncStream::GmailHistory)
            .unwrap()
            .unwrap()
            .token
            .as_deref(),
        Some("h2")
    );
    assert_eq!(recovered.pending_outbox_count().unwrap(), 1);
}

struct DedupSink {
    delivered: Mutex<HashSet<ExternalEventId>>,
    calls: AtomicUsize,
}

#[async_trait]
impl GoogleEventSink for DedupSink {
    async fn deliver(
        &self,
        idempotency_key: ExternalEventId,
        _event: &ExternalEventEnvelope,
        _cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.delivered.lock().unwrap().insert(idempotency_key);
        Ok(())
    }
}

#[tokio::test]
async fn outbox_redelivery_after_crash_is_idempotent_and_acknowledged() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let cursor = store
        .initialize_cursor(fixture.account, SyncStream::GmailHistory, Some("h1"), 1)
        .unwrap();
    store
        .commit(SyncCommit {
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            expected_cursor_token: cursor.token,
            expected_cursor_version: cursor.version,
            successor_cursor_token: "h2".into(),
            cursor_generation: 1,
            events: vec![(
                fixture.event("v1", 1_000),
                ProjectionWrite {
                    json: serde_json::json!({"subject":"subject"}),
                    tombstone: false,
                },
            )],
            committed_at_ms: 1_000,
        })
        .unwrap();
    let store = Arc::new(Mutex::new(store));
    let sink = Arc::new(DedupSink {
        delivered: Mutex::new(HashSet::new()),
        calls: AtomicUsize::new(0),
    });

    // Simulate delivery followed by a crash before acknowledgement.
    let claim = store
        .lock()
        .unwrap()
        .claim_outbox("crashed", 2_000, 1_000, 1)
        .unwrap()
        .pop()
        .unwrap();
    sink.deliver(claim.event.id, &claim.event, &CancellationToken::new())
        .await
        .unwrap();

    let dispatcher =
        GoogleEventDispatcher::new(store.clone(), sink.clone(), "restarted".into(), 1_000).unwrap();
    let outcome = dispatcher
        .dispatch_due(3_000, 10, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome.delivered, 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
    assert_eq!(sink.delivered.lock().unwrap().len(), 1);
    assert_eq!(store.lock().unwrap().pending_outbox_count().unwrap(), 0);
}

struct ScriptedPoller {
    results: Mutex<VecDeque<Result<GooglePollBatch, GooglePollFailure>>>,
    calls: AtomicUsize,
}

#[async_trait]
impl GoogleSyncPoller for ScriptedPoller {
    async fn poll(
        &self,
        _principal: &PrincipalId,
        _cursor: &executive::r#impl::google::GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(result) = self.results.lock().unwrap().pop_front() {
            return result;
        }
        cancel.cancelled().await;
        Err(GooglePollFailure::Transient {
            retry_after_ms: Some(0),
        })
    }
}

fn manager_config() -> GoogleSyncManagerConfig {
    GoogleSyncManagerConfig {
        lease_duration_ms: 1_000,
        idle_poll_ms: 10,
        base_backoff_ms: 10,
        max_backoff_ms: 100,
        circuit_failure_threshold: 2,
    }
}

#[tokio::test]
async fn manager_recovers_offline_state_commits_cursor_and_shuts_down_promptly() {
    let fixture = Fixture::new();
    let store = Arc::new(Mutex::new(fixture.store()));
    let poller = Arc::new(ScriptedPoller {
        results: Mutex::new(VecDeque::from([
            Err(GooglePollFailure::Transient {
                retry_after_ms: Some(0),
            }),
            Err(GooglePollFailure::Transient {
                retry_after_ms: Some(0),
            }),
            Ok(GooglePollBatch {
                successor_cursor: "h2".into(),
                cursor_generation: 1,
                events: vec![(
                    fixture.event("v1", 1_000),
                    ProjectionWrite {
                        json: serde_json::json!({"subject":"subject"}),
                        tombstone: false,
                    },
                )],
            }),
        ])),
        calls: AtomicUsize::new(0),
    });
    let mut manager = GoogleSyncManager::new(
        store.clone(),
        "manager".into(),
        Arc::new(TestClock::new(1_000, 0)),
        manager_config(),
    )
    .unwrap();
    manager
        .register(GoogleSyncRegistration {
            principal: PrincipalId("owner".into()),
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            initial_cursor: Some("h1".into()),
            cursor_generation: 1,
            poller: poller.clone(),
        })
        .unwrap();
    let handle = manager.start(&CancellationToken::new());
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if store
                .lock()
                .unwrap()
                .cursor(fixture.account, SyncStream::GmailHistory)
                .unwrap()
                .unwrap()
                .token
                .as_deref()
                == Some("h2")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(poller.calls.load(Ordering::SeqCst) >= 3);
    let cursor = store
        .lock()
        .unwrap()
        .cursor(fixture.account, SyncStream::GmailHistory)
        .unwrap()
        .unwrap();
    assert_eq!(cursor.health_state, "healthy");
    assert_eq!(cursor.retry_count, 0);
    tokio::time::timeout(std::time::Duration::from_millis(200), handle.shutdown())
        .await
        .unwrap();
}

#[tokio::test]
async fn auth_required_and_revocation_stop_polling_immediately() {
    for failure in [GooglePollFailure::AuthRequired, GooglePollFailure::Revoked] {
        let fixture = Fixture::new();
        let store = Arc::new(Mutex::new(fixture.store()));
        let poller = Arc::new(ScriptedPoller {
            results: Mutex::new(VecDeque::from([Err(failure)])),
            calls: AtomicUsize::new(0),
        });
        let mut manager = GoogleSyncManager::new(
            store.clone(),
            "manager".into(),
            Arc::new(TestClock::new(1_000, 0)),
            manager_config(),
        )
        .unwrap();
        manager
            .register(GoogleSyncRegistration {
                principal: PrincipalId("owner".into()),
                account_id: fixture.account,
                stream: SyncStream::GmailHistory,
                initial_cursor: Some("h1".into()),
                cursor_generation: 1,
                poller: poller.clone(),
            })
            .unwrap();
        let handle = manager.start(&CancellationToken::new());
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        handle.shutdown().await;
        assert_eq!(poller.calls.load(Ordering::SeqCst), 1);
        let health = store
            .lock()
            .unwrap()
            .cursor(fixture.account, SyncStream::GmailHistory)
            .unwrap()
            .unwrap()
            .health_state;
        assert_eq!(
            health,
            if failure == GooglePollFailure::AuthRequired {
                "auth_required"
            } else {
                "revoked"
            }
        );
    }
}

struct ConcurrencyPoller {
    active: AtomicUsize,
    max_active: AtomicUsize,
    calls: AtomicUsize,
}

#[async_trait]
impl GoogleSyncPoller for ConcurrencyPoller {
    async fn poll(
        &self,
        _principal: &PrincipalId,
        cursor: &executive::r#impl::google::GoogleSyncCursor,
        cancel: &CancellationToken,
    ) -> Result<GooglePollBatch, GooglePollFailure> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = tokio::time::sleep(std::time::Duration::from_millis(30)) => {}
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        let current = cursor
            .token
            .as_deref()
            .and_then(|token| token.strip_prefix('h'))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(GooglePollBatch {
            successor_cursor: format!("h{}", current + 1),
            cursor_generation: cursor.generation,
            events: Vec::new(),
        })
    }
}

#[tokio::test]
async fn two_managers_never_poll_the_same_stream_concurrently() {
    let fixture = Fixture::new();
    let store = Arc::new(Mutex::new(fixture.store()));
    let poller = Arc::new(ConcurrencyPoller {
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    });
    let parent = CancellationToken::new();
    let mut handles = Vec::new();
    for owner in ["manager-a", "manager-b"] {
        let mut manager = GoogleSyncManager::new(
            store.clone(),
            owner.into(),
            Arc::new(TestClock::new(1_000, 0)),
            manager_config(),
        )
        .unwrap();
        manager
            .register(GoogleSyncRegistration {
                principal: PrincipalId("owner".into()),
                account_id: fixture.account,
                stream: SyncStream::GmailHistory,
                initial_cursor: Some("h1".into()),
                cursor_generation: 1,
                poller: poller.clone(),
            })
            .unwrap();
        handles.push(manager.start(&parent));
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while poller.calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    parent.cancel();
    for handle in handles {
        handle.shutdown().await;
    }
    assert_eq!(poller.max_active.load(Ordering::SeqCst), 1);
}
