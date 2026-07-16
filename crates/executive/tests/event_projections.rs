use executive::r#impl::events::{
    agent_tree_projection::AgentTreeProjection, debug_projection::DebugProjection,
    memory_job_projection::MemoryJobProjection, metrics_projection::MetricsProjection,
    session_projection::SessionProjection,
};
use executive::service::event_projection::{
    EventProjection, EventProjectionSink, SqliteProjectionStore,
};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventPosition, EventTreeId, EventVisibility, ItemId, ItemPayload, ItemRecord, NamespaceId,
    ParentEventId, SchemaId, SessionId, SpineEvent, TreeSequence, TurnId, SESSION_SCHEMA_VERSION,
};

fn event(
    tree: EventTreeId,
    sequence: u64,
    schema: &str,
    visibility: EventVisibility,
    payload: EventPayload,
    parent: Option<EventId>,
) -> SpineEvent {
    let envelope_payload = match &payload {
        EventPayload::Inline { value } => value.clone(),
        EventPayload::RawObservationRef { uri, .. } => serde_json::json!({"reference": uri}),
    };
    let envelope = EnvelopeV2::new(
        SchemaId(schema.into()),
        EnvelopeV2Target("fixture".into()),
        EnvelopeV2Target("projection".into()),
        EnvelopeV2Delivery::Direct,
        NamespaceId("fixture".into()),
        envelope_payload,
    );
    SpineEvent {
        position: EventPosition {
            tree_id: tree,
            event_id: EventId::new(),
            parent: parent.map(ParentEventId),
            sequence: TreeSequence(sequence),
        },
        identity: EventIdentity {
            root_session_id: "session".into(),
            session_id: "session".into(),
            agent_id: None,
        },
        schema: envelope.schema.clone(),
        visibility,
        envelope,
        payload,
    }
}

fn item(sequence: u64, payload: ItemPayload) -> serde_json::Value {
    serde_json::to_value(ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId::new(),
        session_id: SessionId("session".into()),
        turn_id: TurnId::new(),
        sequence,
        created_at_ms: 1000 + sequence,
        payload,
    })
    .unwrap()
}

#[test]
fn session_memory_debug_and_metrics_rebuild_from_the_same_events() {
    let tree = EventTreeId::for_root_session("session");
    let first = event(
        tree,
        1,
        SchemaId::TURN_EVENT_V1,
        EventVisibility::ModelVisible,
        EventPayload::Inline {
            value: item(
                1,
                ItemPayload::UserMessage {
                    content: "secret user text".into(),
                },
            ),
        },
        None,
    );
    let first_id = first.position.event_id;
    let events = vec![
        first,
        event(
            tree,
            2,
            SchemaId::TURN_EVENT_V1,
            EventVisibility::ModelVisible,
            EventPayload::Inline {
                value: item(
                    2,
                    ItemPayload::ToolResult {
                        call_id: "call".into(),
                        content: "raw tool content".into(),
                        is_error: true,
                        permit_id: None,
                        audit_id: None,
                    },
                ),
            },
            Some(first_id),
        ),
        event(
            tree,
            3,
            SchemaId::EVENT_TOOL_OBSERVATION_V1,
            EventVisibility::Sensitive,
            EventPayload::RawObservationRef {
                uri: "artifact://raw/1".into(),
                media_type: "application/json".into(),
                sha256: "a".repeat(64),
                size_bytes: 50_000,
            },
            None,
        ),
    ];
    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let session = store.advance(&SessionProjection, &events).unwrap().0;
    assert_eq!(session.sessions["session"].items.len(), 2);
    let memory = store.advance(&MemoryJobProjection, &events).unwrap().0;
    assert_eq!(memory.eligible.len(), 1);
    assert_eq!(memory.eligible[0].source_sequence, 2);
    let debug = store.advance(&DebugProjection, &events).unwrap().0;
    assert_eq!(debug.edges.len(), 2);
    let debug_json = serde_json::to_string(&debug).unwrap();
    assert!(!debug_json.contains("secret user text"));
    assert!(!debug_json.contains("raw tool content"));
    assert!(!debug_json.contains("artifact://raw/1"));
    let metrics = store.advance(&MetricsProjection, &events).unwrap().0;
    assert_eq!(metrics.turn_count, 1);
    assert_eq!(metrics.tool_errors, 1);

    let rebuilt = store.rebuild(&SessionProjection, &events).unwrap();
    let incremental = store.advance(&SessionProjection, &events).unwrap();
    assert_eq!(rebuilt, incremental);
}

#[test]
fn agent_projection_rebuilds_parent_child_terminal_state() {
    let tree = EventTreeId::new();
    let agent = uuid::Uuid::new_v4().to_string();
    let parent = uuid::Uuid::new_v4().to_string();
    let events = vec![
        event(
            tree,
            1,
            SchemaId::EVENT_AGENT_STARTED_V1,
            EventVisibility::Control,
            EventPayload::Inline {
                value: serde_json::json!({"agent_id": agent, "parent_agent_id": parent}),
            },
            None,
        ),
        event(
            tree,
            2,
            SchemaId::EVENT_AGENT_STOPPED_V1,
            EventVisibility::Control,
            EventPayload::Inline {
                value: serde_json::json!({"agent_id": agent, "parent_agent_id": parent}),
            },
            None,
        ),
    ];
    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let (state, incremental) = store.advance(&AgentTreeProjection, &events).unwrap();
    assert_eq!(
        state.agents[&agent].parent_agent_id.as_deref(),
        Some(parent.as_str())
    );
    assert_eq!(state.agents[&agent].status, "stopped");
    let (rebuilt, rebuilt_checkpoint) = store.rebuild(&AgentTreeProjection, &events).unwrap();
    assert_eq!(rebuilt, state);
    assert_eq!(rebuilt_checkpoint, incremental);
}

#[test]
fn every_default_projection_has_a_distinct_descriptor() {
    let descriptors = [
        SessionProjection.descriptor(),
        DebugProjection.descriptor(),
        MemoryJobProjection.descriptor(),
        AgentTreeProjection.descriptor(),
        MetricsProjection.descriptor(),
    ];
    let names: std::collections::BTreeSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.name)
        .collect();
    assert_eq!(names.len(), descriptors.len());
}

#[test]
fn default_projection_set_reports_lag_and_poison_without_stopping_peers() {
    let tree = EventTreeId::new();
    let malformed = event(
        tree,
        1,
        SchemaId::TURN_EVENT_V1,
        EventVisibility::ModelVisible,
        EventPayload::Inline {
            value: serde_json::json!({"malformed": true}),
        },
        None,
    );
    let projections = executive::r#impl::events::DefaultEventProjectionSet::in_memory();
    let report = projections.project(&malformed);

    assert!(!report.failures.is_empty());
    assert_eq!(report.poisons.len(), report.failures.len());
    assert!(report
        .lags
        .iter()
        .filter(|lag| report
            .failures
            .iter()
            .any(|failure| failure.projection == lag.projection))
        .all(|lag| lag.pending_events == 1));
    assert!(
        !report.checkpoints.is_empty(),
        "reducers that do not parse the malformed schema must still advance"
    );
}
