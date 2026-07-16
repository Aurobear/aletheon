use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use fabric::{Subsystem, SubsystemContext};
use mnemosyne::{
    CoreMemory, DefaultMemoryService, EpisodicMemory, ExperienceEvent, FactStore, ForgetAuthority,
    ForgetPolicy, ForgetSelector, MemoryMetadata, MemoryRecordId, MemoryScope, MemoryService,
    RecallMemory, RecallRequest, RetentionRepository,
};
use serde_json::Value;
use tokio::sync::Mutex;

fn clock() -> Arc<dyn fabric::Clock> {
    Arc::new(aletheon_kernel::chronos::TestClock::default())
}

async fn service(root: &Path) -> DefaultMemoryService {
    let clock = clock();
    let recall = Arc::new(Mutex::new(
        RecallMemory::new(&root.join("recall.db"), clock.clone()).unwrap(),
    ));
    let facts = Arc::new(Mutex::new(FactStore::open(&root.join("facts.db")).unwrap()));
    let core = Arc::new(Mutex::new(CoreMemory::new()));
    let mut episodic = EpisodicMemory::new(root.join("episodic.db"), clock.clone());
    episodic
        .init(&SubsystemContext {
            name: "forgetting-contract".into(),
            working_dir: root.into(),
            config: Value::Null,
            bus: None,
        })
        .await
        .unwrap();
    DefaultMemoryService::new(recall, facts, core, Arc::new(Mutex::new(episodic)), clock)
        .with_retention_repository(Arc::new(
            RetentionRepository::open(root.join("retention.db")).unwrap(),
        ))
}

fn exact(
    request_id: &str,
    record_id: &str,
    scope: MemoryScope,
    authority: ForgetAuthority,
) -> ForgetPolicy {
    ForgetPolicy {
        request_id: request_id.into(),
        selector: ForgetSelector::Exact {
            record_ids: vec![MemoryRecordId(record_id.into())],
            within: scope,
        },
        requester: "owner".into(),
        reason: "user requested erasure".into(),
        authority,
    }
}

#[tokio::test]
async fn exact_forget_changes_recall_is_idempotent_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path()).await;
    svc.record(ExperienceEvent::Message {
        session: "session-a".into(),
        role: "user".into(),
        content: "remember the copper lighthouse".into(),
        metadata: MemoryMetadata::local("message-1", "event-1", DateTime::<Utc>::UNIX_EPOCH),
    })
    .await
    .unwrap();
    assert_eq!(
        svc.recall(RecallRequest::bounded("session-a", "copper lighthouse"))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let policy = exact(
        "forget-1",
        "message-1",
        MemoryScope::Session("session-a".into()),
        ForgetAuthority::Ordinary,
    );
    let first = svc.forget(policy.clone()).await.unwrap();
    assert_eq!(first.tombstoned, vec![MemoryRecordId("message-1".into())]);
    assert_eq!(svc.forget(policy).await.unwrap(), first);
    assert!(svc
        .recall(RecallRequest::bounded("session-a", "copper lighthouse"))
        .await
        .unwrap()
        .items
        .is_empty());
    drop(svc);
    let reopened = service(dir.path()).await;
    assert!(reopened
        .recall(RecallRequest::bounded("session-a", "copper lighthouse"))
        .await
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
async fn bounded_scope_and_elevated_authority_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path()).await;
    for (id, title) in [("decision-1", "first"), ("decision-2", "second")] {
        svc.record(ExperienceEvent::ArchitectureDecision {
            title: title.into(),
            content: format!("{title} global decision"),
            metadata: MemoryMetadata::local(id, id, DateTime::<Utc>::UNIX_EPOCH),
        })
        .await
        .unwrap();
    }
    let ordinary = exact(
        "global-forget",
        "decision-1",
        MemoryScope::Global,
        ForgetAuthority::Ordinary,
    );
    assert_eq!(
        svc.forget(ordinary).await.unwrap().denied,
        vec![MemoryRecordId("decision-1".into())]
    );

    let unbounded = ForgetPolicy {
        request_id: "bad".into(),
        selector: ForgetSelector::Scope {
            scope: MemoryScope::Global,
            limit: 0,
        },
        requester: "owner".into(),
        reason: "bad selector".into(),
        authority: ForgetAuthority::Ordinary,
    };
    assert!(unbounded.validate().is_err());

    let elevated = exact(
        "elevated-forget",
        "decision-1",
        MemoryScope::Global,
        ForgetAuthority::Elevated {
            proof: "constitutional-admin-receipt".into(),
        },
    );
    assert!(
        svc.forget(elevated.clone()).await.is_err(),
        "preview is mandatory"
    );
    let preview = svc.preview_forget(&elevated).unwrap();
    assert!(preview.denied.is_empty());
    assert_eq!(
        svc.forget(elevated).await.unwrap().remote_pending,
        vec![MemoryRecordId("decision-1".into())]
    );
}
