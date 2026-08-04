use fabric::protocol::memory::{
    MemoryLifecycleStateV1, MemoryObservationKindV1, MemorySensitivityV1,
};
use mnemosyne::{
    GovernedMemoryObservation, MemoryIntakeError, MemoryIntakeLedger, MemoryLifecycleUpdate,
    WorkspaceMemoryKey,
};

fn observation(
    id: &str,
    kind: MemoryObservationKindV1,
    observed_at_ms: i64,
) -> GovernedMemoryObservation {
    GovernedMemoryObservation {
        observation_id: id.into(),
        client_session_id: "client-session".into(),
        client_turn_id: None,
        kind,
        content: format!("governed content {id}"),
        content_fingerprint: format!("keyed-sha256:{}", "a".repeat(64)),
        scrub_policy_version: 1,
        scrub_redactions: 0,
        occurred_at: None,
        source_refs: vec![format!("evidence:{id}")],
        sensitivity: MemorySensitivityV1::Internal,
        explicit_user_action: false,
        principal_id: "principal-a".into(),
        workspace_key: WorkspaceMemoryKey::from_verified("ws:repo:sha256:a").unwrap(),
        connection_kind: "versioned_local_rpc".into(),
        observed_at_ms,
    }
}

#[test]
fn claim_is_single_owner_and_atomically_advances_to_evaluating() {
    let ledger = MemoryIntakeLedger::open_in_memory().unwrap();
    ledger
        .observe(&observation(
            "obs-1",
            MemoryObservationKindV1::TaskOutcome,
            500,
        ))
        .unwrap();

    let claim = ledger
        .claim_next_maintenance("worker-a", 1_000, 5_000)
        .unwrap()
        .unwrap();
    assert_eq!(claim.lifecycle.state, MemoryLifecycleStateV1::Evaluating);
    assert_eq!(claim.lifecycle.revision, 2);
    assert!(ledger
        .claim_next_maintenance("worker-b", 1_001, 5_000)
        .unwrap()
        .is_none());
    let status = ledger.maintenance_status(1_001).unwrap();
    assert_eq!(status.pending_items, 1);
    assert_eq!(status.active_leases, 1);
    assert_eq!(status.expired_leases, 0);
    assert_eq!(status.oldest_pending_age_ms, Some(501));
}

#[test]
fn expired_lease_is_recovered_after_restart_without_duplicate_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("intake.db");
    let ledger = MemoryIntakeLedger::open(&path).unwrap();
    ledger
        .observe(&observation(
            "obs-1",
            MemoryObservationKindV1::TaskOutcome,
            500,
        ))
        .unwrap();
    let first = ledger
        .claim_next_maintenance("worker-a", 1_000, 1_000)
        .unwrap()
        .unwrap();
    assert!(ledger
        .claim_next_maintenance("worker-b", 1_999, 1_000)
        .unwrap()
        .is_none());
    drop(ledger);

    let reopened = MemoryIntakeLedger::open(path).unwrap();
    let recovered = reopened
        .claim_next_maintenance("worker-b", 2_000, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.lifecycle.revision, first.lifecycle.revision);
    assert_ne!(recovered.lease.lease_token, first.lease.lease_token);
    assert_eq!(recovered.lease.owner_id, "worker-b");
}

#[test]
fn settlement_is_terminal_atomic_and_idempotent() {
    let ledger = MemoryIntakeLedger::open_in_memory().unwrap();
    ledger
        .observe(&observation(
            "obs-1",
            MemoryObservationKindV1::TaskOutcome,
            500,
        ))
        .unwrap();
    let claim = ledger
        .claim_next_maintenance("worker-a", 1_000, 5_000)
        .unwrap()
        .unwrap();
    let mut update =
        MemoryLifecycleUpdate::new(claim.lifecycle.revision, MemoryLifecycleStateV1::Rejected);
    update.reason_codes = vec!["score_below_threshold".into()];
    update.terminal_at = Some("2026-08-01T00:00:00Z".into());
    update.created_at_ms = 1_100;
    let receipt = ledger
        .settle_maintenance(&claim.lease, "settlement-1", update.clone(), 1_100)
        .unwrap();
    assert_eq!(receipt.state, MemoryLifecycleStateV1::Rejected);
    assert_eq!(receipt.revision, 3);
    assert_eq!(
        ledger
            .settle_maintenance(&claim.lease, "settlement-1", update.clone(), 1_200)
            .unwrap(),
        receipt
    );
    let mut conflicting = update;
    conflicting.reason_codes = vec!["different".into()];
    assert!(matches!(
        ledger.settle_maintenance(&claim.lease, "settlement-1", conflicting, 1_200),
        Err(MemoryIntakeError::SettlementConflict)
    ));
    assert!(ledger
        .claim_next_maintenance("worker-b", 1_200, 5_000)
        .unwrap()
        .is_none());
}

#[test]
fn deferral_prevents_hot_loop_and_feedback_has_priority() {
    let ledger = MemoryIntakeLedger::open_in_memory().unwrap();
    ledger
        .observe(&observation(
            "normal-first",
            MemoryObservationKindV1::TaskOutcome,
            100,
        ))
        .unwrap();
    ledger
        .observe(&observation(
            "feedback-second",
            MemoryObservationKindV1::Feedback,
            200,
        ))
        .unwrap();
    let feedback = ledger
        .claim_next_maintenance("worker-a", 1_000, 5_000)
        .unwrap()
        .unwrap();
    assert_eq!(feedback.observation.observation_id, "feedback-second");
    ledger
        .defer_maintenance(
            &feedback.lease,
            4_000,
            "semantic_runtime_unavailable",
            1_100,
        )
        .unwrap();
    let normal = ledger
        .claim_next_maintenance("worker-a", 1_101, 5_000)
        .unwrap()
        .unwrap();
    assert_eq!(normal.observation.observation_id, "normal-first");
    assert!(ledger
        .claim_next_maintenance("worker-b", 3_999, 5_000)
        .unwrap()
        .is_none());
    let recovered = ledger
        .claim_next_maintenance("worker-b", 4_000, 5_000)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.observation.observation_id, "feedback-second");
    assert_eq!(recovered.lifecycle.revision, 2);
}
