use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use executive::r#impl::gbrain::GbrainWorker;
use mnemosyne::supplemental::{
    SupplementalDocument, SupplementalSpool, RetryPolicy, SpoolLimits, SupplementalErrorCategory, SupplementalHit,
    SupplementalMemoryTransport, SupplementalTransportError,
};
use mnemosyne::MemorySensitivity;
use tokio_util::sync::CancellationToken;

struct FakeTransport {
    outcomes: Mutex<VecDeque<Result<Option<String>, SupplementalTransportError>>>,
    delivered: Mutex<Vec<SupplementalDocument>>,
    queue_depth: AtomicUsize,
    wait_for_cancel: bool,
}

impl FakeTransport {
    fn with(outcomes: Vec<Result<Option<String>, SupplementalTransportError>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            delivered: Mutex::new(Vec::new()),
            queue_depth: AtomicUsize::new(0),
            wait_for_cancel: false,
        }
    }

    fn cancelling() -> Self {
        Self {
            wait_for_cancel: true,
            ..Self::with(Vec::new())
        }
    }
}

#[async_trait]
impl SupplementalMemoryTransport for FakeTransport {
    fn set_queue_depth(&self, depth: usize) {
        self.queue_depth.store(depth, Ordering::SeqCst);
    }

    async fn put_page(
        &self,
        page: &SupplementalDocument,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, SupplementalTransportError> {
        self.delivered.lock().unwrap().push(page.clone());
        if self.wait_for_cancel {
            cancel.cancelled().await;
            return Err(error(SupplementalErrorCategory::Cancelled));
        }
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None))
    }

    async fn query(
        &self,
        _query: &str,
        _source_id: &str,
        _limit: usize,
        _cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError> {
        Ok(Vec::new())
    }
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
        _cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError> {
        Ok(Vec::new())
    }
    async fn get_page(
        &self,
        _slug: &str,
        _cancel: &CancellationToken,
    ) -> Result<String, SupplementalTransportError> {
        Err(error(SupplementalErrorCategory::Unsupported))
    }
}

fn error(category: SupplementalErrorCategory) -> SupplementalTransportError {
    SupplementalTransportError::new(category, "sanitized")
}

fn spool(dir: &tempfile::TempDir) -> Arc<SupplementalSpool> {
    Arc::new(
        SupplementalSpool::open(
            dir.path().join("spool.db"),
            SpoolLimits {
                max_items: 100,
                max_bytes: 1_000_000,
            },
        )
        .unwrap(),
    )
}

fn enqueue(spool: &SupplementalSpool, id: usize) {
    spool
        .enqueue(
            &format!("goal-{id}"),
            &SupplementalDocument {
                slug: format!("aletheon/goal/{id}"),
                content: format!("goal outcome {id}"),
            },
            MemorySensitivity::Internal,
            0,
        )
        .unwrap();
}

fn worker(
    spool: Arc<SupplementalSpool>,
    transport: Arc<FakeTransport>,
    id: &str,
    batch: usize,
) -> GbrainWorker<FakeTransport> {
    GbrainWorker::new(
        spool,
        transport,
        RetryPolicy {
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            max_attempts: 3,
            max_age_secs: 60,
        },
        id,
        batch,
        100,
    )
    .unwrap()
}

#[tokio::test]
async fn successful_delivery_acknowledges_receipt_and_bounds_batch() {
    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    enqueue(&spool, 1);
    enqueue(&spool, 2);
    let transport = Arc::new(FakeTransport::with(vec![Ok(Some("receipt-1".into()))]));
    let worker = worker(spool.clone(), transport.clone(), "worker", 1);
    let report = worker
        .drain_once(0, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.claimed, 1);
    assert_eq!(report.delivered, 1);
    assert!(spool.has_receipt("goal-1").unwrap());
    assert_eq!(report.queue_depth, 1);
    assert_eq!(transport.queue_depth.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn transient_failure_retries_and_permanent_failure_dead_letters() {
    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    enqueue(&spool, 1);
    enqueue(&spool, 2);
    let transport = Arc::new(FakeTransport::with(vec![
        Err(error(SupplementalErrorCategory::Provider)),
        Err(error(SupplementalErrorCategory::InvalidPage)),
    ]));
    let worker = worker(spool.clone(), transport, "worker", 2);
    let report = worker
        .drain_once(0, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.retried, 1);
    assert_eq!(report.dead_lettered, 1);
    assert_eq!(
        spool.dead_letters(10).unwrap()[0].reason_category,
        "invalid_page"
    );
    assert!(spool.claim("probe", 99, 100, 10).unwrap().is_empty());
    assert_eq!(spool.claim("probe", 125, 100, 10).unwrap().len(), 1);
}

#[tokio::test]
async fn startup_resumes_expired_lease_after_previous_process_crash() {
    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    enqueue(&spool, 1);
    let old = spool.claim("old-process", 0, 10, 1).unwrap();
    assert_eq!(old.len(), 1);
    let transport = Arc::new(FakeTransport::with(vec![Ok(None)]));
    let worker = worker(spool.clone(), transport, "new-process", 10);
    let report = worker
        .drain_once(10, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.delivered, 1);
    assert_eq!(spool.queue_depth().unwrap(), 0);
}

#[tokio::test]
async fn cancellation_leaves_claimed_work_durable_for_lease_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    enqueue(&spool, 1);
    let transport = Arc::new(FakeTransport::cancelling());
    let worker = Arc::new(worker(spool.clone(), transport, "worker", 1));
    let cancel = CancellationToken::new();
    let task = {
        let worker = worker.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { worker.drain_once(0, &cancel).await.unwrap() })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.cancel();
    let report = task.await.unwrap();
    assert_eq!(report.interrupted, 1);
    assert_eq!(spool.queue_depth().unwrap(), 1);
    assert!(spool.claim("new-worker", 99, 100, 1).unwrap().is_empty());
    assert_eq!(spool.claim("new-worker", 100, 100, 1).unwrap().len(), 1);
}

#[tokio::test]
async fn supervised_run_honors_shutdown_without_poll_delay() {
    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    let transport = Arc::new(FakeTransport::with(Vec::new()));
    let worker = worker(spool, transport, "worker", 1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    tokio::time::timeout(
        Duration::from_millis(50),
        worker.run(
            Arc::new(kernel::chronos::TestClock::default()),
            Duration::from_secs(60),
            cancel,
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn retention_tombstone_outbox_is_projected_and_settled_asynchronously() {
    use chrono::{DateTime, Utc};
    use mnemosyne::{
        ForgetAuthority, ForgetPolicy, ForgetSelector, MemoryAuthority, MemoryKind, MemoryMetadata,
        MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus, RetentionRepository,
    };

    let dir = tempfile::tempdir().unwrap();
    let spool = spool(&dir);
    let retention = Arc::new(RetentionRepository::open(dir.path().join("retention.db")).unwrap());
    let record = MemoryRecord {
        id: MemoryRecordId("fact-remote".into()),
        kind: MemoryKind::SemanticFact,
        scope: MemoryScope::Global,
        content: "project a durable tombstone".into(),
        metadata: MemoryMetadata::local("fact-remote", "event-remote", DateTime::<Utc>::UNIX_EPOCH),
        status: MemoryStatus::Current,
        authority: MemoryAuthority::VerifiedLocalSemantic,
        source_event_ids: vec!["event-remote".into()],
        tags: Vec::new(),
    };
    retention.register(&record, 0).unwrap();
    let policy = ForgetPolicy {
        request_id: "forget-remote".into(),
        selector: ForgetSelector::Exact {
            record_ids: vec![record.id.clone()],
            within: MemoryScope::Global,
        },
        requester: "owner".into(),
        reason: "remote propagation".into(),
        authority: ForgetAuthority::Elevated {
            proof: "admin-proof".into(),
        },
    };
    retention.preview_forget(&policy, 1).unwrap();
    assert_eq!(
        retention.forget(&policy, 1).unwrap().remote_pending,
        vec![record.id.clone()]
    );
    let transport = Arc::new(FakeTransport::with(vec![Ok(Some("remote-page".into()))]));
    let worker =
        worker(spool, transport.clone(), "worker", 1).with_retention_repository(retention.clone());
    let report = worker
        .drain_once(2, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.delivered, 1);
    assert!(transport.delivered.lock().unwrap()[0]
        .content
        .contains("Tombstone"));
    assert!(retention.pending_remote_records(10).unwrap().is_empty());
}

#[test]
fn executive_worker_only_schedules_mnemosyne_reconciliation() {
    let source = include_str!("../src/adapters/gbrain/worker.rs");
    for forbidden in [
        ".claim(",
        ".acknowledge(",
        ".retry(",
        "RemoteMemoryReceipt",
        "mark_remote_settled",
        "DeadLettered",
    ] {
        assert!(
            !source.contains(forbidden),
            "Executive retained GBrain memory-domain operation: {forbidden}"
        );
    }
    assert!(source.contains("SupplementalReconciliationService"));
}
