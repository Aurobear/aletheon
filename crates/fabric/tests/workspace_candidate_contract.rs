use fabric::{
    AgoraSpaceId, ContentId, MonoDeadline, MonoTime, ProcessId, SalienceVector, VisibilityScope,
    WallTime, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation, WorkspaceProvenance,
    WORKSPACE_SCHEMA_V1,
};
use uuid::Uuid;

fn candidate() -> WorkspaceCandidate {
    let source = ProcessId(Uuid::from_u128(2));
    WorkspaceCandidate {
        schema_version: WORKSPACE_SCHEMA_V1,
        id: ContentId(Uuid::from_u128(1)),
        space: AgoraSpaceId("space".into()),
        source,
        turn: None,
        content: WorkspaceContent::Observation(WorkspaceObservation {
            what: "disk pressure".into(),
            source: "kernel".into(),
            data: serde_json::json!({"percent": 90}),
        }),
        confidence: 0.9,
        salience: SalienceVector {
            urgency: 0.8,
            goal_relevance: 0.4,
            self_relevance: 0.2,
            novelty: 0.7,
            confidence: 0.9,
            prediction_error: 0.1,
            affect_intensity: 0.3,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec!["trace://disk/1".into()],
            observed_at: WallTime(10),
        },
        visibility: VisibilityScope::Session,
        dependencies: Vec::new(),
        created_at: MonoTime(100),
        expires_at: Some(MonoDeadline(MonoTime(200))),
    }
}

#[test]
fn contract_round_trips_stable_tagged_content() {
    let value = candidate();
    value.validate().unwrap();
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(json["content"]["kind"], "observation");
    assert_eq!(json["visibility"]["kind"], "session");
    let decoded: WorkspaceCandidate = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.id, value.id);
    assert_eq!(decoded.space, value.space);
}

#[test]
fn validation_rejects_source_confidence_salience_lifecycle_and_schema_errors() {
    let mut cases = Vec::new();
    let mut value = candidate();
    value.schema_version = 99;
    cases.push(value);
    let mut value = candidate();
    value.provenance.producer = ProcessId(Uuid::from_u128(99));
    cases.push(value);
    let mut value = candidate();
    value.confidence = f32::NAN;
    cases.push(value);
    let mut value = candidate();
    value.salience.urgency = 1.1;
    cases.push(value);
    let mut value = candidate();
    value.provenance.source_refs.clear();
    cases.push(value);
    let mut value = candidate();
    value.expires_at = Some(MonoDeadline(value.created_at));
    cases.push(value);
    let mut value = candidate();
    value.dependencies.push(value.id);
    cases.push(value);
    for invalid in cases {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn validation_rejects_unversioned_or_oversized_extensions() {
    let mut value = candidate();
    value.content = WorkspaceContent::Extension {
        schema: "custom".into(),
        payload: serde_json::json!({"x": 1}),
    };
    assert!(value.validate().is_err());
    value.content = WorkspaceContent::Extension {
        schema: "v1/custom".into(),
        payload: serde_json::json!({"x": "x".repeat(70_000)}),
    };
    assert!(value.validate().is_err());
}

#[test]
fn fingerprint_ignores_arrival_identity_and_salience_but_not_content() {
    let first = candidate();
    let mut equivalent = candidate();
    equivalent.id = ContentId(Uuid::from_u128(55));
    equivalent.created_at = MonoTime(999);
    equivalent.salience.urgency = 0.1;
    assert_eq!(
        first.content_fingerprint().unwrap(),
        equivalent.content_fingerprint().unwrap()
    );
    if let WorkspaceContent::Observation(observation) = &mut equivalent.content {
        observation.what = "network pressure".into();
    }
    assert_ne!(
        first.content_fingerprint().unwrap(),
        equivalent.content_fingerprint().unwrap()
    );
}
