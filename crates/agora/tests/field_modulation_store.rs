use agora::SqliteBroadcastStore;
use fabric::{
    AgoraSpaceId, ConsciousArbitrationMode, ConsciousTraceEvent, FieldDecisionKind,
    FieldDecisionReason,
};

fn modulation(metric_ref: &str) -> ConsciousTraceEvent {
    ConsciousTraceEvent::FieldModulation {
        mode: ConsciousArbitrationMode::Enforce,
        decision: FieldDecisionKind::Defer,
        reason: FieldDecisionReason::Negated,
        operation_id: "operation-1".into(),
        call_id: "call-1".into(),
        broadcast_epoch: Some(7),
        baseline: None,
        effective: Some(0.8),
        delta: None,
        metric_ref: metric_ref.into(),
    }
}

#[test]
fn field_modulation_is_durable_idempotent_and_conflict_safe() {
    let store = SqliteBroadcastStore::open_in_memory().unwrap();
    let space = AgoraSpaceId("session-1".into());
    let event = modulation("metric-1");

    store.save_field_modulation(&space, &event).unwrap();
    store.save_field_modulation(&space, &event).unwrap();

    assert_eq!(store.field_modulations(&space).unwrap(), vec![event]);
    assert!(store
        .save_field_modulation(&space, &modulation("metric-conflict"))
        .is_err());
}
