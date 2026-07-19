use dasein::core::store::SelfFieldStore;
use dasein::core::{SelfField, SelfFieldConfig};
use dasein::dasein::ledger::SelfLedger;
use dasein::dasein::sorge::SystemSorgeTimer;
use dasein::dasein::{DaseinModule, DaseinRuntimeConfig};
use fabric::dasein::{
    ExperienceProvenance, ExperienceSource, InterpretedExperience, SelfEventId,
    SelfTransitionRequest, SelfVersion,
};
use fabric::{Subsystem, SubsystemContext, WallTime};
use std::path::Path;
use std::sync::Arc;

fn open_store(path: &Path) -> Arc<SelfFieldStore> {
    Arc::new(SelfFieldStore::new(path.to_path_buf()).unwrap())
}

fn module_with_ledger(
    store: Arc<SelfFieldStore>,
    clock: Arc<kernel::chronos::TestClock>,
) -> DaseinModule {
    DaseinModule::with_runtime_and_ledger(
        clock,
        Arc::new(SystemSorgeTimer),
        DaseinRuntimeConfig::default(),
        Some(Arc::new(SelfLedger::new(store))),
    )
    .unwrap()
    .0
}

fn request(
    event_id: SelfEventId,
    expected_version: u64,
    observed_at: i64,
    content: InterpretedExperience,
) -> SelfTransitionRequest {
    SelfTransitionRequest {
        event_id,
        source: ExperienceSource::Runtime,
        observed_at: WallTime(observed_at),
        content,
        provenance: ExperienceProvenance {
            producer: "ledger-replay-test".into(),
            session_id: None,
            turn_id: None,
            source_ref: None,
        },
        expected_version: SelfVersion(expected_version),
    }
}

async fn seed_three_events(module: &DaseinModule) -> Vec<SelfTransitionRequest> {
    let requests = vec![
        request(
            SelfEventId::new(),
            0,
            10,
            InterpretedExperience::WorldEntityObserved {
                entity_id: "compiler".into(),
                what_it_is: "build tool".into(),
                for_the_sake_of: Vec::new(),
                readiness: fabric::dasein::ReadinessState::ReadyToHand,
            },
        ),
        request(
            SelfEventId::new(),
            1,
            20,
            InterpretedExperience::Lived {
                semantic: "building the project".into(),
                action: Some("compile".into()),
                perception: None,
            },
        ),
        request(
            SelfEventId::new(),
            2,
            30,
            InterpretedExperience::ReadinessChanged {
                entity_id: "compiler".into(),
                old_state: fabric::dasein::ReadinessState::ReadyToHand,
                new_state: fabric::dasein::ReadinessState::PresentAtHand,
            },
        ),
    ];
    for request in &requests {
        module.transition(request.clone()).await.unwrap();
    }
    requests
}

#[tokio::test]
async fn ledger_append_reopen_and_duplicate_are_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    let event = request(
        SelfEventId::new(),
        0,
        10,
        InterpretedExperience::Lived {
            semantic: "durable event".into(),
            action: None,
            perception: None,
        },
    );
    let first = module.transition(event.clone()).await.unwrap();
    drop(module);

    let reopened = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(reopened.replay_durable_state().await.unwrap(), 1);
    let duplicate = reopened.transition(event).await.unwrap();
    assert_eq!(first, duplicate);
    let count: u64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM self_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(reopened.temporality().current_position().0, 1);
}

#[tokio::test]
async fn replay_restores_context_and_version_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&original).await;
    let expected_context = serde_json::to_vec(&original.to_context_injection()).unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(replayed.replay_durable_state().await.unwrap(), 3);
    assert_eq!(replayed.self_version().await, SelfVersion(3));
    assert_eq!(
        serde_json::to_vec(&replayed.to_context_injection()).unwrap(),
        expected_context
    );
}

#[tokio::test]
async fn replay_verifies_checkpoint_prefix_then_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    let seeded = seed_three_events(&original).await;
    original.checkpoint_durable_state().unwrap();
    original
        .transition(request(
            SelfEventId::new(),
            3,
            40,
            InterpretedExperience::KnowledgeAsserted {
                assertions: vec!["checkpoint suffix".into()],
                confidence: 0.9,
            },
        ))
        .await
        .unwrap();
    let expected = serde_json::to_vec(&original.to_context_injection()).unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    assert_eq!(replayed.replay_durable_state().await.unwrap(), 4);
    assert_eq!(
        serde_json::to_vec(&replayed.to_context_injection()).unwrap(),
        expected
    );
    assert_eq!(seeded.len(), 3);
}

#[tokio::test]
async fn ledger_corruption_fails_replay_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&module).await;
    store
        .conn()
        .execute(
            "UPDATE self_events SET checksum = 'tampered' WHERE seq = 2",
            [],
        )
        .unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    let error = replayed.replay_durable_state().await.unwrap_err();
    assert!(error.to_string().contains("checksum"));
    assert_eq!(replayed.self_version().await, SelfVersion(0));
}

#[tokio::test]
async fn replay_rejects_corrupt_checkpoint_even_when_ledger_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let store = open_store(&path);
    let module = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    seed_three_events(&module).await;
    module.checkpoint_durable_state().unwrap();
    store
        .conn()
        .execute("UPDATE self_snapshots SET checksum = 'tampered'", [])
        .unwrap();

    let replayed = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(200, 0)),
    );
    let error = replayed.replay_durable_state().await.unwrap_err();
    assert!(error.to_string().contains("snapshot checksum"));
    assert_eq!(replayed.self_version().await, SelfVersion(0));
}

#[tokio::test]
async fn restart_replays_then_records_one_resumption_experience() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self.db");
    let original = module_with_ledger(
        open_store(&path),
        Arc::new(kernel::chronos::TestClock::new(100, 0)),
    );
    original
        .transition(request(
            SelfEventId::new(),
            0,
            100,
            InterpretedExperience::Lived {
                semantic: "before restart".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();

    let store = open_store(&path);
    let restarted = module_with_ledger(
        store.clone(),
        Arc::new(kernel::chronos::TestClock::new(5_100, 0)),
    );
    dasein::dasein::persistence::load_dasein_state(&restarted, &store)
        .await
        .unwrap();

    assert_eq!(restarted.self_version().await, SelfVersion(2));
    assert_eq!(restarted.temporality().current_position().0, 2);
    let events = SelfLedger::new(store).load_verified().unwrap();
    assert!(matches!(
        events.last().unwrap().request.content,
        InterpretedExperience::ResumedAfterInterval { elapsed_ms: 5_000 }
    ));
}

#[tokio::test]
async fn restart_self_field_replays_before_start_and_checkpoints_after_stop() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self-field.db");
    let context = SubsystemContext {
        name: "self-field-ledger-test".into(),
        working_dir: temp.path().to_path_buf(),
        config: serde_json::Value::Null,
        bus: None,
    };

    let mut first = SelfField::new(SelfFieldConfig {
        db_path: Some(path.clone()),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(100, 0))),
        ..Default::default()
    });
    first.init(&context).await.unwrap();
    first
        .dasein()
        .unwrap()
        .record_outcome(
            "first installed lifecycle",
            fabric::dasein::OutcomeStatus::Succeeded,
            "self-field-ledger-test",
        )
        .await
        .unwrap();
    first.shutdown().await.unwrap();

    let mut restarted = SelfField::new(SelfFieldConfig {
        db_path: Some(path),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(5_100, 0))),
        ..Default::default()
    });
    restarted.init(&context).await.unwrap();
    let dasein = restarted.dasein().unwrap();
    assert!(dasein.is_alive());
    assert_eq!(dasein.self_version().await, SelfVersion(2));
    assert_eq!(dasein.temporality().current_position().0, 2);
    restarted.shutdown().await.unwrap();
}
