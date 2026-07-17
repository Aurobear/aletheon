use chrono::{DateTime, Duration, Utc};
use mnemosyne::{
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryRecord, MemoryRecordId, MemoryScope,
    MemoryStatus, ScopeAncestry,
};

fn ancestry() -> ScopeAncestry {
    ScopeAncestry {
        principal_id: Some("principal-a".into()),
        session_id: Some("session-a".into()),
        goal_id: Some("goal-a".into()),
        agent_id: Some("agent-a".into()),
        task_id: Some("task-a".into()),
    }
}

fn record() -> MemoryRecord {
    let metadata = MemoryMetadata::local("record-a", "event-a", DateTime::<Utc>::UNIX_EPOCH);
    MemoryRecord {
        id: MemoryRecordId("record-a".into()),
        kind: MemoryKind::SemanticFact,
        scope: MemoryScope::Task("task-a".into()),
        content: "canonical payload".into(),
        metadata,
        status: MemoryStatus::Current,
        authority: MemoryAuthority::VerifiedLocalSemantic,
        source_event_ids: vec!["event-a".into()],
        tags: vec!["architecture".into()],
    }
}

#[test]
fn scope_visibility_requires_exact_ancestry_member() {
    let chain = ancestry();
    for scope in [
        MemoryScope::Global,
        MemoryScope::Principal("principal-a".into()),
        MemoryScope::Session("session-a".into()),
        MemoryScope::Goal("goal-a".into()),
        MemoryScope::Agent("agent-a".into()),
        MemoryScope::Task("task-a".into()),
    ] {
        assert!(scope.allows(&chain), "expected {scope:?} to be visible");
    }
    assert!(!MemoryScope::Session("session-b".into()).allows(&chain));
    assert!(!MemoryScope::Agent("agent-b".into()).allows(&chain));
    assert!(!MemoryScope::Task("task-b".into()).allows(&chain));
}

#[test]
fn scope_serialization_is_stable_and_validated() {
    let json = serde_json::to_string(&MemoryScope::Session("session-a".into())).unwrap();
    assert_eq!(json, r#"{"kind":"session","id":"session-a"}"#);
    assert!(MemoryScope::Agent(" ".into()).validate().is_err());
}

#[test]
fn record_serialization_round_trip_is_stable() {
    let expected = record();
    expected.validate().unwrap();
    let json = serde_json::to_string(&expected).unwrap();
    assert!(json.contains(r#""kind":"semantic_fact""#));
    assert!(json.contains(r#""authority":"verified_local_semantic""#));
    assert_eq!(
        serde_json::from_str::<MemoryRecord>(&json).unwrap(),
        expected
    );
}

#[test]
fn record_validation_rejects_invalid_identity_content_and_metadata() {
    let mut value = record();
    value.id.0.clear();
    assert!(value.validate().is_err());

    let mut value = record();
    value.content = "x".repeat(MemoryRecord::MAX_CONTENT_BYTES + 1);
    assert!(value.validate().is_err());

    let mut value = record();
    value.metadata.confidence = f64::NAN;
    assert!(value.validate().is_err());

    let mut value = record();
    let start = DateTime::<Utc>::UNIX_EPOCH;
    value.metadata.valid_from = Some(start + Duration::seconds(1));
    value.metadata.valid_until = Some(start);
    assert!(value.validate().is_err());
}

#[test]
fn record_recall_projection_preserves_authority() {
    let expected = record();
    let projected = mnemosyne::RecallItem::from_record(expected.clone()).unwrap();
    assert_eq!(projected.authority, MemoryAuthority::VerifiedLocalSemantic);
    let restored = projected
        .into_record(MemoryKind::SemanticFact, MemoryScope::Task("task-a".into()))
        .unwrap();
    assert_eq!(restored.authority, expected.authority);
    assert_eq!(restored.id, expected.id);
}
