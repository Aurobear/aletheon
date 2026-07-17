use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventTreeId, EventVisibility, NamespaceId, ParentEventId, SchemaId, UnsequencedEvent,
};

fn event(schema: &str, payload: EventPayload) -> UnsequencedEvent {
    UnsequencedEvent {
        tree_id: EventTreeId::new(),
        event_id: EventId::new(),
        parent: None,
        identity: EventIdentity {
            root_session_id: "root".into(),
            session_id: "session".into(),
            agent_id: Some("agent".into()),
        },
        envelope: EnvelopeV2::new(
            SchemaId(schema.into()),
            EnvelopeV2Target("executive".into()),
            EnvelopeV2Target("session:session".into()),
            EnvelopeV2Delivery::Direct,
            NamespaceId("session:session".into()),
            serde_json::json!({"kind": "turn_started"}),
        ),
        visibility: EventVisibility::Control,
        payload,
    }
}

#[test]
fn validates_identity_target_parent_and_known_schema() {
    let mut candidate = event(
        SchemaId::TURN_EVENT_V1,
        EventPayload::Inline {
            value: serde_json::json!({"turn": "t1"}),
        },
    );
    assert!(candidate.validate().is_ok());

    candidate.parent = Some(ParentEventId(candidate.event_id));
    assert!(candidate
        .validate()
        .unwrap_err()
        .to_string()
        .contains("causal parent"));

    candidate.parent = None;
    candidate.envelope.schema = SchemaId("unknown/v9".into());
    assert!(candidate
        .validate()
        .unwrap_err()
        .to_string()
        .contains("unsupported schema"));
}

#[test]
fn raw_observations_are_references_and_never_model_visible() {
    let mut candidate = event(
        SchemaId::EVENT_TOOL_OBSERVATION_V1,
        EventPayload::RawObservationRef {
            uri: "artifact://observations/1".into(),
            media_type: "application/json".into(),
            sha256: "a".repeat(64),
            size_bytes: 4096,
        },
    );
    assert!(candidate.validate().is_ok());
    candidate.visibility = EventVisibility::ModelVisible;
    assert!(candidate
        .validate()
        .unwrap_err()
        .to_string()
        .contains("model-visible"));
}
