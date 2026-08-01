use fabric::protocol::memory::{
    MemoryIntakeStatusV1, MemoryLifecycleStateV1, MemoryObservationKindV1, MemorySensitivityV1,
};
use mnemosyne::{
    GovernedMemoryObservation, MemoryIntakeLedger, MemoryLifecycleUpdate, WorkspaceMemoryKey,
};

fn observation(principal: &str, workspace: &str, content: &str) -> GovernedMemoryObservation {
    GovernedMemoryObservation {
        observation_id: "obs-1".into(),
        client_session_id: "client-session-1".into(),
        client_turn_id: Some("turn-1".into()),
        kind: MemoryObservationKindV1::TaskOutcome,
        content: content.into(),
        occurred_at: Some("2026-08-01T00:00:00Z".into()),
        source_refs: vec!["terminal-receipt:1".into()],
        sensitivity: MemorySensitivityV1::Internal,
        explicit_user_action: false,
        principal_id: principal.into(),
        workspace_key: WorkspaceMemoryKey::from_verified(workspace).unwrap(),
        connection_kind: "versioned_local_rpc".into(),
        observed_at_ms: 1_785_552_000_000,
    }
}

#[test]
fn observation_insert_is_durable_idempotent_and_conflict_safe() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("intake.db");
    let ledger = MemoryIntakeLedger::open(&path).unwrap();
    let value = observation("principal-a", "ws:repo:sha256:a", "tests passed");

    let first = ledger.observe(&value).unwrap();
    assert_eq!(first.intake_status, MemoryIntakeStatusV1::Observed);
    let duplicate = ledger.observe(&value).unwrap();
    assert_eq!(duplicate.intake_status, MemoryIntakeStatusV1::Duplicate);
    assert_eq!(duplicate.durable_intake_id, first.durable_intake_id);

    let conflicting = observation("principal-a", "ws:repo:sha256:a", "different payload");
    assert!(ledger.observe(&conflicting).is_err());
    drop(ledger);

    let reopened = MemoryIntakeLedger::open(&path).unwrap();
    let after_restart = reopened.observe(&value).unwrap();
    assert_eq!(after_restart.intake_status, MemoryIntakeStatusV1::Duplicate);
    assert_eq!(after_restart.durable_intake_id, first.durable_intake_id);
}

#[test]
fn observation_and_receipt_authority_are_principal_and_workspace_scoped() {
    let ledger = MemoryIntakeLedger::open_in_memory().unwrap();
    let first = ledger
        .observe(&observation(
            "principal-a",
            "ws:repo:sha256:a",
            "same payload",
        ))
        .unwrap();
    let other_principal = ledger
        .observe(&observation(
            "principal-b",
            "ws:repo:sha256:a",
            "same payload",
        ))
        .unwrap();
    let other_workspace = ledger
        .observe(&observation(
            "principal-a",
            "ws:repo:sha256:b",
            "same payload",
        ))
        .unwrap();

    assert_ne!(first.durable_intake_id, other_principal.durable_intake_id);
    assert_ne!(first.durable_intake_id, other_workspace.durable_intake_id);
    assert!(ledger
        .receipt("principal-b", "ws:repo:sha256:a", &first.durable_intake_id)
        .unwrap()
        .is_none());
    assert!(ledger
        .receipt("principal-a", "ws:repo:sha256:b", &first.durable_intake_id)
        .unwrap()
        .is_none());
    assert_eq!(
        ledger
            .receipt("principal-a", "ws:repo:sha256:a", &first.durable_intake_id)
            .unwrap()
            .unwrap()
            .state,
        MemoryLifecycleStateV1::Observed
    );
}

#[test]
fn lifecycle_revisions_are_monotonic_and_projection_failure_preserves_local_result() {
    let ledger = MemoryIntakeLedger::open_in_memory().unwrap();
    let intake = ledger
        .observe(&observation(
            "principal-a",
            "ws:repo:sha256:a",
            "verified constraint",
        ))
        .unwrap();

    let evaluating = ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            MemoryLifecycleUpdate::new(1, MemoryLifecycleStateV1::Evaluating),
        )
        .unwrap();
    assert_eq!(evaluating.revision, 2);

    let mut promotion = MemoryLifecycleUpdate::new(2, MemoryLifecycleStateV1::PromotedLocal);
    promotion.resulting_record_ids = vec!["record-local-1".into()];
    let promoted = ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            promotion,
        )
        .unwrap();
    assert_eq!(promoted.revision, 3);

    let queued = ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            MemoryLifecycleUpdate::new(3, MemoryLifecycleStateV1::ProjectionQueued),
        )
        .unwrap();
    let failed = ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            MemoryLifecycleUpdate::new(queued.revision, MemoryLifecycleStateV1::ProjectionFailed),
        )
        .unwrap();
    assert_eq!(failed.resulting_record_ids, vec!["record-local-1"]);

    assert!(ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            MemoryLifecycleUpdate::new(failed.revision, MemoryLifecycleStateV1::Observed),
        )
        .is_err());
    assert!(ledger
        .transition(
            "principal-a",
            "ws:repo:sha256:a",
            &intake.durable_intake_id,
            MemoryLifecycleUpdate::new(1, MemoryLifecycleStateV1::ProjectionQueued),
        )
        .is_err());
}
