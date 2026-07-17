use executive::r#impl::events::{DefaultEventProjectionSet, SqliteEventSpine};
use executive::r#impl::session::{
    canonical_store::CanonicalSessionStore, event_sourced_store::reconcile_committed_session_events,
};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventSpine, EventTreeId, EventVisibility, ItemId, ItemPayload, ItemRecord, MessageId,
    NamespaceId, SchemaId, SessionAppendStore, SessionId, SessionRecord, SessionStatus, TurnId,
    UnsequencedEvent, SESSION_SCHEMA_VERSION,
};
use tempfile::tempdir;
use uuid::Uuid;

fn committed_event(
    session_id: &SessionId,
    event_id: u128,
    schema: &'static str,
    visibility: EventVisibility,
    value: serde_json::Value,
) -> UnsequencedEvent {
    let event_id = EventId(Uuid::from_u128(event_id));
    let mut envelope = EnvelopeV2::new(
        SchemaId(schema.into()),
        EnvelopeV2Target("session-command".into()),
        EnvelopeV2Target(format!("session:{}", session_id.0)),
        EnvelopeV2Delivery::Direct,
        NamespaceId(format!("session:{}", session_id.0)),
        value.clone(),
    );
    envelope.id = MessageId(event_id.0);
    UnsequencedEvent {
        tree_id: EventTreeId::for_root_session(&session_id.0),
        event_id,
        parent: None,
        identity: EventIdentity {
            root_session_id: session_id.0.clone(),
            session_id: session_id.0.clone(),
            agent_id: None,
        },
        envelope,
        visibility,
        payload: EventPayload::Inline { value },
    }
}

#[tokio::test]
async fn restart_reconciles_committed_session_events_missing_from_read_model() {
    let directory = tempdir().unwrap();
    let event_path = directory.path().join("events.db");
    let projection_path = directory.path().join("event-projections.db");
    let session_path = directory.path().join("sessions.db");
    let session_id = SessionId("orphaned-session".into());
    let session = SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: session_id.clone(),
        parent: None,
        created_at_ms: 10,
        status: SessionStatus::Active,
    };
    let item = ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(Uuid::from_u128(3)),
        session_id: session_id.clone(),
        turn_id: TurnId(Uuid::from_u128(4)),
        sequence: 1,
        created_at_ms: 11,
        payload: ItemPayload::UserMessage {
            content: "survive restart".into(),
        },
    };

    // Simulate a process dying after the authoritative event commit and before
    // either deterministic projections or the Session read model are updated.
    {
        let spine = SqliteEventSpine::open(&event_path).unwrap();
        spine
            .append(committed_event(
                &session_id,
                1,
                SchemaId::EVENT_SESSION_CREATED_V1,
                EventVisibility::Control,
                serde_json::to_value(&session).unwrap(),
            ))
            .unwrap();
        spine
            .append(committed_event(
                &session_id,
                2,
                SchemaId::TURN_EVENT_V1,
                EventVisibility::ModelVisible,
                serde_json::to_value(&item).unwrap(),
            ))
            .unwrap();
    }

    let spine = SqliteEventSpine::open(&event_path).unwrap();
    let projections = DefaultEventProjectionSet::open(&projection_path).unwrap();
    let store = CanonicalSessionStore::open(&session_path).unwrap();
    assert!(store.load_session(&session_id).await.unwrap().is_none());

    let first = reconcile_committed_session_events(&spine, &projections, &store)
        .await
        .unwrap();
    assert_eq!(first.scanned, 2);
    assert_eq!(first.materialized, 2);
    assert_eq!(
        store.load_session(&session_id).await.unwrap(),
        Some(session)
    );
    assert_eq!(
        store.load_items(&session_id, None).await.unwrap(),
        vec![item]
    );

    // A later restart replays the same committed prefix without duplicating
    // items or rejecting already-advanced projection checkpoints.
    drop(store);
    drop(projections);
    drop(spine);
    let spine = SqliteEventSpine::open(&event_path).unwrap();
    let projections = DefaultEventProjectionSet::open(&projection_path).unwrap();
    let store = CanonicalSessionStore::open(&session_path).unwrap();
    let second = reconcile_committed_session_events(&spine, &projections, &store)
        .await
        .unwrap();
    assert_eq!(second.scanned, 2);
    assert_eq!(store.load_items(&session_id, None).await.unwrap().len(), 1);
}
