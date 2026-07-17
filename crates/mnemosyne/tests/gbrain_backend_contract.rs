use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mnemosyne::backends::gbrain::{
    EnqueueOutcome, GbrainBackend, GbrainBackendConfig, GbrainPage, GbrainSpool, SpoolLimits,
    SupplementalErrorCategory, SupplementalHit, SupplementalMemoryTransport,
    SupplementalTransportError,
};
use mnemosyne::{
    ExperienceEvent, ForgetPolicy, GbrainDegradedCategory, MemoryKindLabel, MemoryMetadata,
    MemoryProvenance, MemorySensitivity, RecallRequest, RecallSourceLabel, TemporalState,
};
use tokio_util::sync::CancellationToken;

struct FakeTransport {
    query: Mutex<Result<Vec<SupplementalHit>, SupplementalTransportError>>,
    search: Mutex<Result<Vec<SupplementalHit>, SupplementalTransportError>>,
    page: Mutex<Result<String, SupplementalTransportError>>,
    query_delay: Mutex<Duration>,
    put_calls: AtomicUsize,
    get_calls: AtomicUsize,
    search_calls: AtomicUsize,
}

impl FakeTransport {
    fn healthy() -> Self {
        Self {
            query: Mutex::new(Ok(Vec::new())),
            search: Mutex::new(Ok(Vec::new())),
            page: Mutex::new(Err(error(SupplementalErrorCategory::MalformedResponse))),
            query_delay: Mutex::new(Duration::ZERO),
            put_calls: AtomicUsize::new(0),
            get_calls: AtomicUsize::new(0),
            search_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SupplementalMemoryTransport for FakeTransport {
    async fn put_page(
        &self,
        _page: &GbrainPage,
        _cancel: &CancellationToken,
    ) -> Result<Option<String>, SupplementalTransportError> {
        self.put_calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    async fn query(
        &self,
        _query: &str,
        _source_id: &str,
        _limit: usize,
        _cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError> {
        let delay = *self.query_delay.lock().unwrap();
        tokio::time::sleep(delay).await;
        self.query.lock().unwrap().clone()
    }
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
        _cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError> {
        self.search_calls.fetch_add(1, Ordering::SeqCst);
        self.search.lock().unwrap().clone()
    }
    async fn get_page(
        &self,
        _slug: &str,
        _cancel: &CancellationToken,
    ) -> Result<String, SupplementalTransportError> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        self.page.lock().unwrap().clone()
    }
}

fn error(category: SupplementalErrorCategory) -> SupplementalTransportError {
    SupplementalTransportError::new(category, "sanitized")
}

fn metadata(id: &str) -> MemoryMetadata {
    let now = DateTime::<Utc>::UNIX_EPOCH;
    MemoryMetadata {
        record_id: id.into(),
        provenance: MemoryProvenance {
            source: "aletheon".into(),
            source_id: "adr-1".into(),
            principal: Some("owner".into()),
            source_commit: Some("abc123".into()),
        },
        source_time: Some(now),
        observed_time: now,
        valid_from: Some(now),
        valid_until: None,
        supersedes: None,
        superseded_by: None,
        confidence: 0.9,
        sensitivity: MemorySensitivity::Internal,
    }
}

fn event(id: &str) -> ExperienceEvent {
    ExperienceEvent::ArchitectureDecision {
        title: "Memory boundary".into(),
        content: "Use HTTP MCP.".into(),
        metadata: metadata(id),
    }
}

fn build_backend(
    dir: &tempfile::TempDir,
    transport: Arc<FakeTransport>,
    timeout_ms: u64,
) -> GbrainBackend<FakeTransport> {
    let spool = Arc::new(
        GbrainSpool::open(
            dir.path().join("spool.db"),
            SpoolLimits {
                max_items: 100,
                max_bytes: 1_000_000,
            },
        )
        .unwrap(),
    );
    let config = GbrainBackendConfig {
        enabled: true,
        projection_enabled: true,
        read_sources: vec!["aletheon".into()],
        write_source: "aletheon".into(),
        request_timeout_ms: timeout_ms,
        recall_limit: 4,
        ..Default::default()
    };
    GbrainBackend::new(spool, transport, config)
}

fn request() -> RecallRequest {
    let mut request = RecallRequest::bounded("s", "memory boundary");
    request.current_at = Some(DateTime::<Utc>::UNIX_EPOCH);
    request
}

#[test]
fn record_commits_locally_without_transport_and_is_stably_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    let backend = build_backend(&dir, transport.clone(), 100);
    assert_eq!(
        backend.record(&event("decision-1"), 0).unwrap(),
        EnqueueOutcome::Inserted
    );
    assert_eq!(
        backend.record(&event("decision-1"), 1).unwrap(),
        EnqueueOutcome::AlreadyPresent
    );
    assert_eq!(transport.put_calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.spool().queue_depth().unwrap(), 1);

    let message = ExperienceEvent::Message {
        session: "s".into(),
        role: "user".into(),
        content: "raw".into(),
        metadata: metadata("message-1"),
    };
    assert_eq!(
        backend.record(&message, 2).unwrap(),
        EnqueueOutcome::ExcludedSensitive
    );
    let mut restricted = event("decision-secret");
    if let ExperienceEvent::ArchitectureDecision { metadata, .. } = &mut restricted {
        metadata.sensitivity = MemorySensitivity::Restricted;
    }
    assert_eq!(
        backend.record(&restricted, 3).unwrap(),
        EnqueueOutcome::ExcludedSensitive
    );
}

#[tokio::test]
async fn recall_preserves_provenance_temporal_fields_and_uses_get_only_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    let page = GbrainPage::from_event(&event("decision-1"))
        .unwrap()
        .unwrap();
    *transport.query.lock().unwrap() = Ok(vec![SupplementalHit {
        source_id: "aletheon".into(),
        slug: page.slug.clone(),
        content: page.content.clone(),
        score: 0.9,
    }]);
    let backend = build_backend(&dir, transport.clone(), 100);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert!(!recall.health.degraded);
    assert_eq!(recall.items.len(), 1);
    assert_eq!(recall.items[0].metadata.record_id, "decision-1");
    assert_eq!(
        recall.items[0].metadata.provenance.source_commit.as_deref(),
        Some("abc123")
    );
    assert_eq!(recall.items[0].temporal_state, TemporalState::Current);
    assert_eq!(transport.get_calls.load(Ordering::SeqCst), 0);
    let snapshot = backend.metrics().snapshot();
    assert_eq!(
        snapshot.memory_recall_hits[&RecallSourceLabel::Gbrain]
            [&MemoryKindLabel::ExternalReference],
        1
    );
    assert_eq!(
        snapshot.memory_recall_latency_ms[&RecallSourceLabel::Gbrain].count,
        1
    );

    *transport.query.lock().unwrap() = Ok(vec![SupplementalHit {
        source_id: "aletheon".into(),
        slug: page.slug,
        content: String::new(),
        score: 0.9,
    }]);
    *transport.page.lock().unwrap() = Ok(page.content);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert_eq!(recall.items.len(), 1);
    assert_eq!(transport.get_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn query_failure_falls_back_to_verified_search() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    let page = GbrainPage::from_event(&event("decision-1"))
        .unwrap()
        .unwrap();
    *transport.query.lock().unwrap() = Err(error(SupplementalErrorCategory::Provider));
    *transport.search.lock().unwrap() = Ok(vec![SupplementalHit {
        source_id: "aletheon".into(),
        slug: page.slug,
        content: page.content,
        score: 0.8,
    }]);
    let backend = build_backend(&dir, transport.clone(), 100);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert_eq!(recall.items.len(), 1);
    assert_eq!(transport.search_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn outage_slow_and_malformed_remote_memory_degrade_to_empty() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    *transport.query.lock().unwrap() = Err(error(SupplementalErrorCategory::Transport));
    *transport.search.lock().unwrap() = Err(error(SupplementalErrorCategory::Provider));
    let backend = build_backend(&dir, transport, 100);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert!(recall.items.is_empty());
    assert!(recall.health.degraded);
    assert_eq!(
        recall.health.error_category,
        Some(SupplementalErrorCategory::Provider)
    );
    assert_eq!(
        backend.metrics().snapshot().memory_gbrain_degraded[&GbrainDegradedCategory::Provider],
        1
    );

    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    *transport.query_delay.lock().unwrap() = Duration::from_millis(100);
    let backend = build_backend(&dir, transport, 5);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert!(recall.items.is_empty());
    assert_eq!(
        recall.health.error_category,
        Some(SupplementalErrorCategory::Timeout)
    );

    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::healthy());
    *transport.query.lock().unwrap() = Ok(vec![SupplementalHit {
        source_id: "aletheon".into(),
        slug: "bad".into(),
        content: "---\ninvalid".into(),
        score: 1.0,
    }]);
    let backend = build_backend(&dir, transport, 100);
    let recall = backend.recall(request(), &CancellationToken::new()).await;
    assert!(recall.items.is_empty());
    assert_eq!(
        recall.health.error_category,
        Some(SupplementalErrorCategory::MalformedResponse)
    );
}

#[test]
fn forget_is_explicitly_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let backend = build_backend(&dir, Arc::new(FakeTransport::healthy()), 100);
    assert!(backend
        .forget(ForgetPolicy {
            request_id: "request-1".into(),
            selector: mnemosyne::ForgetSelector::Scope {
                scope: mnemosyne::MemoryScope::Session("s".into()),
                limit: 1,
            },
            requester: "owner".into(),
            reason: "test".into(),
            authority: mnemosyne::ForgetAuthority::Ordinary,
        })
        .unwrap_err()
        .to_string()
        .contains("unsupported"));
}
