use super::*;
use ::contracts::{Subsystem, SubsystemContext};
use std::path::Path;

struct EmptyVectorBackend;

#[async_trait::async_trait]
impl crate::RecallSearchBackend for EmptyVectorBackend {
    async fn search(
        &self,
        _request: &RecallRequest,
        _predicate: &crate::ScopePredicate,
        _top_k: usize,
    ) -> anyhow::Result<crate::SearchOutcome> {
        Ok(crate::SearchOutcome::default())
    }
}

fn test_clock() -> Arc<dyn ::contracts::Clock> {
    Arc::new(kernel::chronos::TestClock::default())
}

fn metadata(id: &str) -> MemoryMetadata {
    MemoryMetadata::local(id, id, DateTime::<Utc>::UNIX_EPOCH)
}

async fn build_service(dir: &Path) -> DefaultMemoryService {
    let clock = test_clock();
    let recall_memory = Arc::new(Mutex::new(
        RecallMemory::new(&dir.join("recall.db"), clock.clone()).unwrap(),
    ));
    let fact_store = Arc::new(Mutex::new(FactStore::open(&dir.join("facts.db")).unwrap()));
    let core_memory = Arc::new(Mutex::new(CoreMemory::new()));
    let mut episodic_memory = EpisodicMemory::new(dir.join("episodic.db"), clock.clone());
    let ctx = SubsystemContext {
        name: "episodic_memory".into(),
        working_dir: dir.to_path_buf(),
        config: serde_json::Value::Null,
    };
    episodic_memory.init(&ctx).await.unwrap();
    let episodic = Arc::new(Mutex::new(episodic_memory));
    DefaultMemoryService::new(recall_memory, fact_store, core_memory, episodic, clock)
}

#[tokio::test]
async fn vector_backend_trust_is_derived_from_exact_endpoint_grant() {
    let grant = crate::credential::EmbeddingCredentialGrant::new(
        "test-principal",
        "https://embedding.example/v1",
        "test-provider",
        100,
        1,
        "secret",
    );
    let dir = tempfile::tempdir().unwrap();
    let rejected = build_service(dir.path()).await.with_vector_search_backend(
        Arc::new(EmptyVectorBackend),
        &grant,
        "https://embedding.example.evil/v1",
        10,
    );
    assert!(!rejected.embedding_endpoint_trusted);

    let dir = tempfile::tempdir().unwrap();
    let approved = build_service(dir.path()).await.with_vector_search_backend(
        Arc::new(EmptyVectorBackend),
        &grant,
        "https://embedding.example/v1",
        10,
    );
    assert!(approved.embedding_endpoint_trusted);
}

#[tokio::test]
async fn record_message_stores_into_recall_memory() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    svc.record(ExperienceEvent::Message {
        session: "s1".into(),
        role: "user".into(),
        content: "hello world".into(),
        metadata: metadata("message-1"),
    })
    .await
    .unwrap();

    let rm = svc.recall_memory.lock().await;
    let count = rm.count().unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn record_reflection_stores_into_episodic_memory() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    svc.record(ExperienceEvent::Reflection {
        content: "learned something".into(),
        metadata: metadata("reflection-1"),
    })
    .await
    .unwrap();

    let episodic = svc.episodic.lock().await;
    assert_eq!(episodic.reflection_count().unwrap(), 1);
}

#[tokio::test]
async fn recall_returns_matching_facts() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    {
        let fact_store = svc.fact_store.lock().await;
        fact_store
            .add_fact("rust is great", "general", "", "test", 0.5, "long", 0)
            .unwrap();
    }

    let result = svc
        .recall(RecallRequest::bounded("s1", "rust"))
        .await
        .unwrap();
    assert_eq!(result.texts(), vec!["rust is great"]);
    let item = &result.items[0];
    assert_eq!(item.metadata.record_id, "mnemosyne:fact:1");
    assert_eq!(item.metadata.provenance.source_id, "1");
    assert_eq!(item.metadata.confidence, 0.5);
    assert_eq!(item.metadata.sensitivity, MemorySensitivity::Internal);
    assert_eq!(item.temporal_state, TemporalState::Unknown);
}

#[tokio::test]
async fn memory_hybrid_flag_off_is_fts_path_equivalent() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    {
        let fact_store = svc.fact_store.lock().await;
        fact_store
            .add_fact("stable lexical", "general", "", "test", 0.5, "long", 0)
            .unwrap();
    }
    let request = RecallRequest::bounded("s1", "stable");
    let prefilter = crate::RecallPreFilter {
        ancestry: crate::ScopeAncestry {
            session_id: Some("s1".into()),
            ..Default::default()
        },
        max_sensitivity: MemorySensitivity::Restricted,
        allowed_authorities: all_memory_authorities(),
    };
    let legacy = svc
        .recall_with_prefilter(request.clone(), &prefilter)
        .await
        .unwrap();
    let flagged_off = svc.recall(request).await.unwrap();
    assert_eq!(flagged_off, legacy);
}

#[tokio::test]
async fn verified_agent_recall_cannot_widen_scope_from_request_or_query() {
    use ::contracts::{AgentId, AgentTaskId, ProcessId};
    use uuid::Uuid;

    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    svc.record(ExperienceEvent::Message {
        session: "parent-session".into(),
        role: "user".into(),
        content: "parent-only marker".into(),
        metadata: metadata("parent-message"),
    })
    .await
    .unwrap();
    let context = crate::AgentMemoryContext::verified(
        ProcessId(Uuid::new_v4()),
        AgentId(Uuid::new_v4()),
        AgentTaskId("child-task".into()),
        "verified-parent-projection",
    )
    .unwrap();
    let result = svc
        .recall_for_agent(
            &context,
            RecallRequest::bounded("parent-session", "parent-only marker global parent session"),
            MemorySensitivity::Internal,
        )
        .await
        .unwrap();
    assert!(result.items.is_empty());
}

#[tokio::test]
async fn governed_fact_recall_materializes_only_allowed_principal_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    {
        let facts = svc.fact_store.lock().await;
        facts
            .add_fact_governed(
                "shared principal marker allowed",
                "general",
                "",
                "principal",
                "explicit",
                "owner-a",
                0.8,
                "long",
                0,
            )
            .unwrap();
        facts
            .add_fact_governed(
                "shared principal marker denied",
                "general",
                "",
                "principal",
                "explicit",
                "owner-b",
                0.8,
                "long",
                0,
            )
            .unwrap();
    }
    let prefilter = crate::RecallPreFilter {
        ancestry: crate::ScopeAncestry {
            principal_id: Some("owner-a".into()),
            ..Default::default()
        },
        max_sensitivity: MemorySensitivity::Internal,
        allowed_authorities: vec![MemoryAuthority::VerifiedLocalSemantic],
    };
    let result = svc
        .recall_with_prefilter(
            RecallRequest::bounded("untrusted-session", "shared principal marker"),
            &prefilter,
        )
        .await
        .unwrap();
    assert_eq!(result.texts(), vec!["shared principal marker allowed"]);
}

#[test]
fn temporal_state_uses_explicit_validity_and_supersession() {
    let now = DateTime::<Utc>::UNIX_EPOCH;
    let mut value = metadata("decision-1");
    assert_eq!(value.temporal_state(Some(now)), TemporalState::Current);
    value.valid_until = Some(now);
    assert_eq!(value.temporal_state(Some(now)), TemporalState::Expired);
    value.superseded_by = Some("decision-2".into());
    assert_eq!(value.temporal_state(Some(now)), TemporalState::Superseded);
    value.superseded_by = None;
    value.valid_until = None;
    assert_eq!(value.temporal_state(None), TemporalState::Unknown);
}

#[test]
fn metadata_round_trip_preserves_contract_fields() {
    let now = DateTime::<Utc>::UNIX_EPOCH;
    let value = MemoryMetadata {
        record_id: "goal:g1:outcome".into(),
        provenance: MemoryProvenance {
            source: "goal_store".into(),
            source_id: "g1".into(),
            principal: Some("owner".into()),
            source_commit: Some("abc123".into()),
        },
        source_time: Some(now),
        observed_time: now,
        valid_from: Some(now),
        valid_until: Some(now + chrono::Duration::days(1)),
        supersedes: Some("goal:g0:outcome".into()),
        superseded_by: None,
        confidence: 0.9,
        sensitivity: MemorySensitivity::Confidential,
    };
    let encoded = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<MemoryMetadata>(&encoded).unwrap(),
        value
    );
}

#[tokio::test]
async fn recall_rejects_unbounded_requests() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    let mut req = RecallRequest::bounded("s1", "rust");
    req.max_items = RecallRequest::MAX_ITEMS + 1;
    assert!(svc.recall(req).await.is_err());
}

#[tokio::test]
async fn consolidate_is_ok_and_forget_fails_without_retention() {
    let dir = tempfile::tempdir().unwrap();
    let svc = build_service(dir.path()).await;
    svc.consolidate(MemoryScope::Global).await.unwrap();
    svc.consolidate(MemoryScope::Session("s1".into()))
        .await
        .unwrap();
    assert!(svc
        .forget(ForgetPolicy {
            request_id: "request-1".into(),
            selector: ForgetSelector::Scope {
                scope: MemoryScope::Session("s1".into()),
                limit: 1,
            },
            requester: "owner".into(),
            reason: "test".into(),
            authority: ForgetAuthority::Ordinary,
        })
        .await
        .is_err());
}
