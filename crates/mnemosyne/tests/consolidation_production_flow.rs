use std::sync::Arc;

use fabric::{Subsystem, SubsystemContext};
use mnemosyne::consolidation::{ConsolidationRepository, ExtractionStatus};
use mnemosyne::{
    CoreMemory, DefaultMemoryService, EpisodicMemory, ExperienceEvent, FactStore, MemoryMetadata,
    MemoryScope, MemoryService, RecallMemory,
};
use tokio::sync::Mutex;

async fn service(
    root: &std::path::Path,
    repository: Arc<ConsolidationRepository>,
) -> DefaultMemoryService {
    let clock: Arc<dyn fabric::Clock> = Arc::new(aletheon_kernel::chronos::TestClock::default());
    let recall = Arc::new(Mutex::new(
        RecallMemory::new(&root.join("recall.db"), clock.clone()).unwrap(),
    ));
    let facts = Arc::new(Mutex::new(FactStore::open(&root.join("facts.db")).unwrap()));
    let core = Arc::new(Mutex::new(CoreMemory::new()));
    let mut episodic = EpisodicMemory::new(root.join("episodic.db"), clock.clone());
    episodic
        .init(&SubsystemContext {
            name: "production-consolidation".into(),
            working_dir: root.into(),
            config: serde_json::Value::Null,
            bus: None,
        })
        .await
        .unwrap();
    DefaultMemoryService::new(recall, facts, core, Arc::new(Mutex::new(episodic)), clock)
        .with_consolidation_repository(repository)
}

#[tokio::test]
async fn production_record_enqueues_extracts_and_consolidates_restart_idempotently() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("consolidation.db");
    let repository = Arc::new(ConsolidationRepository::open(&path).unwrap());
    let service = service(root.path(), repository.clone()).await;
    let event = ExperienceEvent::Message {
        session: "session-a".into(),
        role: "assistant".into(),
        content: "The deployment uses a bounded reconciliation worker.".into(),
        metadata: MemoryMetadata::local(
            "event-production-1",
            "turn-a",
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        ),
    };

    service.record(event.clone()).await.unwrap();
    assert_eq!(
        repository.status("experience:event-production-1").unwrap(),
        ExtractionStatus::Pending
    );
    assert!(repository
        .claim_extraction("premature-worker", 0, 60_000, 60_000)
        .unwrap()
        .is_none());
    service.consolidate(MemoryScope::Global).await.unwrap();
    assert_eq!(
        repository.status("experience:event-production-1").unwrap(),
        ExtractionStatus::Pending,
        "periodic global consolidation must not fabricate session completion"
    );
    assert_eq!(repository.consolidated_record_count().unwrap(), 0);
    service
        .consolidate(MemoryScope::Session("session-a".into()))
        .await
        .unwrap();
    assert_eq!(
        repository.status("experience:event-production-1").unwrap(),
        ExtractionStatus::Succeeded
    );
    assert_eq!(repository.consolidated_record_count().unwrap(), 1);

    // Replaying the durable event cannot create a second job or record.
    service.record(event).await.unwrap();
    service
        .consolidate(MemoryScope::Session("session-a".into()))
        .await
        .unwrap();
    drop(service);
    drop(repository);
    let reopened = ConsolidationRepository::open(&path).unwrap();
    assert_eq!(
        reopened.status("experience:event-production-1").unwrap(),
        ExtractionStatus::Succeeded
    );
    assert_eq!(reopened.consolidated_record_count().unwrap(), 1);
}
