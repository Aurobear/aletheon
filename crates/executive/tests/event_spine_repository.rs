use std::{sync::Arc, thread};

use executive::r#impl::events::{EventReadFilter, SqliteEventSpine};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventSpine, EventTreeId, EventVisibility, NamespaceId, ParentEventId, SchemaId, TreeSequence,
    UnsequencedEvent,
};
use tempfile::tempdir;

fn event(tree_id: EventTreeId, event_id: EventId) -> UnsequencedEvent {
    UnsequencedEvent {
        tree_id,
        event_id,
        parent: None,
        identity: EventIdentity {
            root_session_id: "root".into(),
            session_id: "session".into(),
            agent_id: None,
        },
        envelope: EnvelopeV2::new(
            SchemaId(SchemaId::TURN_EVENT_V1.into()),
            EnvelopeV2Target("turn-coordinator".into()),
            EnvelopeV2Target("session:session".into()),
            EnvelopeV2Delivery::Direct,
            NamespaceId("session:session".into()),
            serde_json::json!({"event": "started"}),
        ),
        visibility: EventVisibility::Control,
        payload: EventPayload::Inline {
            value: serde_json::json!({"turn_id": "turn"}),
        },
    }
}

#[test]
fn append_is_idempotent_and_conflicting_retry_is_rejected() {
    let store = SqliteEventSpine::open(":memory:").unwrap();
    let candidate = event(EventTreeId::new(), EventId::new());
    let first = store.append(candidate.clone()).unwrap();
    let retry = store.append(candidate.clone()).unwrap();
    assert_eq!(first.position, retry.position);

    let mut conflict = candidate;
    conflict.payload = EventPayload::Inline {
        value: serde_json::json!({"different": true}),
    };
    assert!(store
        .append(conflict)
        .unwrap_err()
        .to_string()
        .contains("conflicts"));
}

#[test]
fn allocates_monotonic_tree_order_under_concurrency() {
    let directory = tempdir().unwrap();
    let store = Arc::new(SqliteEventSpine::open(directory.path().join("events.db")).unwrap());
    let tree = EventTreeId::new();
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let store = store.clone();
            thread::spawn(move || store.append(event(tree, EventId::new())).unwrap())
        })
        .collect();
    let mut sequences: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().position.sequence.0)
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=16).collect::<Vec<_>>());
}

#[test]
fn validates_parent_and_reopens_with_bounded_filtered_reads() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("events.db");
    let tree = EventTreeId::new();
    let root_id = EventId::new();
    {
        let store = SqliteEventSpine::open(&path).unwrap();
        store.append(event(tree, root_id)).unwrap();
        let mut child = event(tree, EventId::new());
        child.parent = Some(ParentEventId(root_id));
        child.visibility = EventVisibility::ModelVisible;
        store.append(child).unwrap();

        let mut orphan = event(tree, EventId::new());
        orphan.parent = Some(ParentEventId(EventId::new()));
        assert!(store
            .append(orphan)
            .unwrap_err()
            .to_string()
            .contains("does not exist"));
    }
    let reopened = SqliteEventSpine::open(&path).unwrap();
    let visible = reopened
        .read_tree(
            tree,
            EventReadFilter {
                from_sequence: Some(TreeSequence(2)),
                visibility: Some(EventVisibility::ModelVisible),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].position.sequence, TreeSequence(2));
}
