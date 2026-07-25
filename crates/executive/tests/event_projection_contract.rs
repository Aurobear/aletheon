use executive::application::event_projection::{
    EventProjection, ProjectionDescriptor, ProjectionError, SqliteProjectionStore,
};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventPosition, EventTreeId, EventVisibility, NamespaceId, SchemaId, SpineEvent, TreeSequence,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct CountState {
    values: Vec<u64>,
}

struct CountProjection {
    version: u32,
    poison_value: Option<u64>,
}

impl EventProjection for CountProjection {
    type State = CountState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "count",
            version: self.version,
            accepted_schemas: &[SchemaId::TURN_EVENT_V1],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        let value = event.payload_value();
        if self.poison_value == Some(value) {
            return Err(ProjectionError::InvalidDescriptor("poison fixture".into()));
        }
        state.values.push(value);
        Ok(())
    }
}

trait FixtureValue {
    fn payload_value(&self) -> u64;
}

impl FixtureValue for SpineEvent {
    fn payload_value(&self) -> u64 {
        match &self.payload {
            EventPayload::Inline { value } => value["value"].as_u64().unwrap(),
            EventPayload::RawObservationRef { .. } => panic!("unexpected raw fixture"),
        }
    }
}

fn event(tree: EventTreeId, sequence: u64, value: u64) -> SpineEvent {
    let envelope = EnvelopeV2::new(
        SchemaId(SchemaId::TURN_EVENT_V1.into()),
        EnvelopeV2Target("fixture".into()),
        EnvelopeV2Target("projection".into()),
        EnvelopeV2Delivery::Direct,
        NamespaceId("fixture".into()),
        serde_json::json!({"value": value}),
    );
    SpineEvent {
        position: EventPosition {
            tree_id: tree,
            event_id: EventId::new(),
            parent: None,
            sequence: TreeSequence(sequence),
        },
        identity: EventIdentity {
            root_session_id: "root".into(),
            session_id: "session".into(),
            agent_id: None,
        },
        schema: envelope.schema.clone(),
        visibility: EventVisibility::Control,
        envelope,
        payload: EventPayload::Inline {
            value: serde_json::json!({"value": value}),
        },
    }
}

#[test]
fn duplicate_restart_and_rebuild_are_byte_stable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.db");
    let tree = EventTreeId::new();
    let events = vec![event(tree, 1, 10), event(tree, 2, 20)];
    let projection = CountProjection {
        version: 1,
        poison_value: None,
    };
    let first = SqliteProjectionStore::open(&path).unwrap();
    let (state, checkpoint) = first.advance(&projection, &events).unwrap();
    assert_eq!(state.values, vec![10, 20]);
    let (duplicate, duplicate_checkpoint) = first.advance(&projection, &events).unwrap();
    assert_eq!(duplicate, state);
    assert_eq!(duplicate_checkpoint, checkpoint);
    drop(first);

    let reopened = SqliteProjectionStore::open(&path).unwrap();
    let (restarted, restarted_checkpoint) = reopened.advance(&projection, &events).unwrap();
    assert_eq!(restarted, state);
    assert_eq!(restarted_checkpoint, checkpoint);
    let (rebuilt, rebuilt_checkpoint) = reopened.rebuild(&projection, &events).unwrap();
    assert_eq!(rebuilt, state);
    assert_eq!(rebuilt_checkpoint, checkpoint);
}

#[test]
fn schema_upgrade_requires_explicit_rebuild() {
    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let tree = EventTreeId::new();
    let events = [event(tree, 1, 1)];
    store
        .advance(
            &CountProjection {
                version: 1,
                poison_value: None,
            },
            &events,
        )
        .unwrap();
    assert!(matches!(
        store.advance(
            &CountProjection {
                version: 2,
                poison_value: None,
            },
            &events
        ),
        Err(ProjectionError::VersionMismatch { .. })
    ));
    let (_, checkpoint) = store
        .rebuild(
            &CountProjection {
                version: 2,
                poison_value: None,
            },
            &events,
        )
        .unwrap();
    assert_eq!(checkpoint.version, 2);
}

#[test]
fn poison_is_checkpointed_without_blocking_an_unrelated_projection() {
    let store = SqliteProjectionStore::open(":memory:").unwrap();
    let tree = EventTreeId::new();
    let events = [event(tree, 1, 7)];
    assert!(matches!(
        store.advance(
            &CountProjection {
                version: 1,
                poison_value: Some(7),
            },
            &events
        ),
        Err(ProjectionError::Poisoned { .. })
    ));
    assert_eq!(store.poison("count").unwrap().unwrap().sequence, 1);

    struct Other;
    impl EventProjection for Other {
        type State = CountState;
        fn descriptor(&self) -> ProjectionDescriptor {
            ProjectionDescriptor {
                name: "other",
                version: 1,
                accepted_schemas: &[SchemaId::TURN_EVENT_V1],
            }
        }
        fn apply(
            &self,
            state: &mut Self::State,
            event: &SpineEvent,
        ) -> Result<(), ProjectionError> {
            state.values.push(event.payload_value());
            Ok(())
        }
    }
    assert_eq!(store.advance(&Other, &events).unwrap().0.values, vec![7]);
}
