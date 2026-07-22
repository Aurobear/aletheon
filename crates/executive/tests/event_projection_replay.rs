use std::path::PathBuf;

use executive::runtime::events::{
    agent_tree_projection::AgentTreeProjection, debug_projection::DebugProjection,
    memory_job_projection::MemoryJobProjection, metrics_projection::MetricsProjection,
    session_projection::SessionProjection,
};
use executive::service::event_projection::{EventProjection, SqliteProjectionStore};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventPosition, EventTreeId, EventVisibility, ItemId, ItemPayload, ItemRecord, NamespaceId,
    SchemaId, SessionId, SpineEvent, TreeSequence, TurnId, SESSION_SCHEMA_VERSION,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct FixtureRow {
    sequence: u64,
    schema: String,
    visibility: EventVisibility,
    kind: String,
}

fn fixture() -> Vec<SpineEvent> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/event_spine/cross_domain_v1.jsonl");
    let tree = EventTreeId::for_root_session("fixture-session");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<FixtureRow>(line).unwrap())
        .map(|row| {
            let value = match row.kind.as_str() {
                "session" => serde_json::to_value(fabric::SessionRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: SessionId("fixture-session".into()),
                    parent: None,
                    created_at_ms: 900,
                    status: fabric::SessionStatus::Active,
                })
                .unwrap(),
                "turn" => serde_json::to_value(ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId(uuid::Uuid::from_u128(1)),
                    session_id: SessionId("fixture-session".into()),
                    turn_id: TurnId(uuid::Uuid::from_u128(2)),
                    sequence: 1,
                    created_at_ms: 1_000,
                    payload: ItemPayload::AssistantMessage {
                        content: "public answer".into(),
                    },
                })
                .unwrap(),
                "child_agent" => serde_json::json!({
                    "agent_id": "00000000-0000-0000-0000-000000000003",
                    "parent_agent_id": "00000000-0000-0000-0000-000000000004"
                }),
                "memory_candidate" => serde_json::json!({
                    "record_id": "candidate:fixture",
                    "kind": "fixture_candidate",
                    "content": {"source_event": 1},
                    "sensitivity": "internal"
                }),
                "agora_broadcast" => serde_json::json!({"epoch": 7, "selected": ["candidate"]}),
                "restart" => serde_json::json!({"generation": 2}),
                "raw_tool" => serde_json::json!({"reference": "artifact://secret"}),
                other => panic!("unknown fixture kind {other}"),
            };
            let payload = if row.kind == "raw_tool" {
                EventPayload::RawObservationRef {
                    uri: "artifact://secret".into(),
                    media_type: "application/json".into(),
                    sha256: "a".repeat(64),
                    size_bytes: 100_000,
                }
            } else {
                EventPayload::Inline {
                    value: value.clone(),
                }
            };
            let envelope = EnvelopeV2::new(
                SchemaId(row.schema.clone()),
                EnvelopeV2Target("fixture".into()),
                EnvelopeV2Target("fixture-session".into()),
                EnvelopeV2Delivery::Direct,
                NamespaceId("fixture-session".into()),
                value,
            );
            SpineEvent {
                position: EventPosition {
                    tree_id: tree,
                    event_id: EventId(uuid::Uuid::from_u128(100 + row.sequence as u128)),
                    parent: None,
                    sequence: TreeSequence(row.sequence),
                },
                identity: EventIdentity {
                    root_session_id: "fixture-session".into(),
                    session_id: "fixture-session".into(),
                    agent_id: (row.kind == "child_agent")
                        .then(|| "00000000-0000-0000-0000-000000000003".into()),
                },
                schema: SchemaId(row.schema),
                visibility: row.visibility,
                envelope,
                payload,
            }
        })
        .collect()
}

fn incremental_and_rebuild<P>(projection: &P, events: &[SpineEvent])
where
    P: EventProjection,
    P::State: PartialEq + std::fmt::Debug + serde::Serialize,
{
    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let mut incremental = None;
    for event in events {
        incremental = Some(
            store
                .advance(projection, std::slice::from_ref(event))
                .unwrap(),
        );
    }
    let incremental = incremental.unwrap();
    let rebuilt = store.rebuild(projection, events).unwrap();
    assert_eq!(
        serde_json::to_vec(&incremental).unwrap(),
        serde_json::to_vec(&rebuilt).unwrap()
    );
    assert_eq!(incremental.1.checksum, rebuilt.1.checksum);
    let rebuilt_again = store.rebuild(projection, events).unwrap();
    assert_eq!(
        serde_json::to_vec(&rebuilt).unwrap(),
        serde_json::to_vec(&rebuilt_again).unwrap()
    );
    assert_eq!(rebuilt.1.checksum, rebuilt_again.1.checksum);
}

#[test]
fn recorded_cross_domain_fixture_is_byte_stable_for_every_projection() {
    let events = fixture();
    incremental_and_rebuild(&SessionProjection, &events);
    incremental_and_rebuild(&DebugProjection, &events);
    incremental_and_rebuild(&MemoryJobProjection, &events);
    incremental_and_rebuild(&AgentTreeProjection, &events);
    incremental_and_rebuild(&MetricsProjection, &events);

    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let public = store.rebuild(&SessionProjection, &events).unwrap().0;
    let public_json = serde_json::to_string(&public).unwrap();
    assert!(!public_json.contains("artifact://secret"));
    assert_eq!(public.sessions["fixture-session"].items.len(), 1);
}
