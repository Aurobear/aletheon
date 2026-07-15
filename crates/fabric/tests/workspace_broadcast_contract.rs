use fabric::dasein::SelfVersion;
use fabric::{
    AgoraSpaceId, BroadcastAck, BroadcastAckStatus, BroadcastDelivery, BroadcastEpoch,
    CandidateScore, ContentId, MonoTime, ProcessId, SalienceVector, SelectionExplanation,
    SelectionResult, VisibilityScope, WallTime, WorkspaceBroadcast, WorkspaceCandidate,
    WorkspaceContent, WorkspaceObservation, WorkspaceProvenance, MAX_BROADCAST_RESPONSES,
    WORKSPACE_SCHEMA_V1,
};
use uuid::Uuid;

fn candidate(id: u128) -> WorkspaceCandidate {
    let source = ProcessId(Uuid::from_u128(10));
    WorkspaceCandidate {
        schema_version: WORKSPACE_SCHEMA_V1,
        id: ContentId(Uuid::from_u128(id)),
        space: AgoraSpaceId("space".into()),
        source,
        turn: None,
        content: WorkspaceContent::Observation(WorkspaceObservation {
            what: format!("observation-{id}"),
            source: "fixture".into(),
            data: serde_json::json!({"id": id}),
        }),
        confidence: 1.0,
        salience: SalienceVector {
            urgency: 1.0,
            goal_relevance: 0.0,
            self_relevance: 0.0,
            novelty: 0.0,
            confidence: 0.0,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec![format!("fixture://{id}")],
            observed_at: WallTime(1),
        },
        visibility: VisibilityScope::Session,
        dependencies: Vec::new(),
        created_at: MonoTime(1),
        expires_at: None,
    }
}

fn selection() -> SelectionResult {
    let selected = vec![candidate(1), candidate(2)];
    let selected_ids = selected.iter().map(|value| value.id).collect();
    SelectionResult {
        selected,
        explanation: SelectionExplanation {
            policy_version: 1,
            evaluated: Vec::<CandidateScore>::new(),
            selected_ids,
            rejected_below_ignition: Vec::new(),
        },
    }
}

#[test]
fn broadcast_derives_and_validates_lossless_selection() {
    let value =
        WorkspaceBroadcast::from_selection(BroadcastEpoch(1), selection(), SelfVersion(2), 3)
            .unwrap();
    value.validate().unwrap();
    assert_eq!(value.winner_ids.len(), 2);
    assert_eq!(value.selected.len(), 2);
    assert_eq!(value.checksum().unwrap().len(), 64);

    let mut mismatched = value.clone();
    mismatched.winner_ids.pop();
    assert!(mismatched.validate().is_err());
    let mut cross_space = value;
    cross_space.selected[1].space = AgoraSpaceId("other".into());
    assert!(cross_space.validate().is_err());
}

#[test]
fn acknowledgement_bounds_and_status_are_explicit() {
    let mut ack = BroadcastAck {
        schema_version: WORKSPACE_SCHEMA_V1,
        space: AgoraSpaceId("space".into()),
        epoch: BroadcastEpoch(1),
        processor: ProcessId(Uuid::from_u128(9)),
        response_ids: vec![ContentId(Uuid::from_u128(1))],
        status: BroadcastAckStatus::Responded,
        observed_at: WallTime(3),
        detail: None,
    };
    ack.validate().unwrap();
    ack.response_ids = (0..=MAX_BROADCAST_RESPONSES)
        .map(|id| ContentId(Uuid::from_u128(id as u128 + 1)))
        .collect();
    assert!(ack.validate().is_err());
    ack.response_ids.clear();
    ack.status = BroadcastAckStatus::TimedOut;
    assert!(ack.validate().is_err());
    ack.detail = Some("deadline".into());
    ack.validate().unwrap();
}

#[test]
fn delivery_contract_rejects_visibility_leaks_without_carrying_full_broadcast() {
    let mut private = candidate(1);
    private.visibility = VisibilityScope::PrivateProcess {
        process: ProcessId(Uuid::from_u128(7)),
    };
    let delivery = BroadcastDelivery {
        schema_version: WORKSPACE_SCHEMA_V1,
        epoch: BroadcastEpoch(1),
        space: AgoraSpaceId("space".into()),
        recipient: ProcessId(Uuid::from_u128(8)),
        recipient_agent_root: ProcessId(Uuid::from_u128(8)),
        broadcast_checksum: "0".repeat(64),
        dasein_version: SelfVersion(1),
        workspace_version: 1,
        selected: vec![private],
    };
    assert!(delivery.validate().is_err());
    let json = serde_json::to_value(delivery).unwrap();
    assert!(json.get("broadcast").is_none());
}
