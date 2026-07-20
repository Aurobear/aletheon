use std::{collections::HashSet, sync::Arc};

use executive::{
    r#impl::session::canonical_store::CanonicalSessionStore,
    service::session_service::SessionService,
};
use fabric::{
    protocol::client::{ClientEvent, EventCursor, ItemPhase},
    AppendOutcome, ItemId, ItemPayload, ItemRecord, SessionAppendStore, SessionId, SessionRecord,
    SessionStatus, TurnId, SESSION_SCHEMA_VERSION,
};
use tokio::sync::Mutex;

async fn fixture() -> (Arc<dyn SessionAppendStore>, SessionService, SessionId) {
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let session_id = SessionId("daemon-protocol-reconnect".into());
    store
        .create(SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: session_id.clone(),
            parent: None,
            created_at_ms: 1,
            status: SessionStatus::Active,
        })
        .await
        .unwrap();
    let service = SessionService::new(store.clone(), Arc::new(Mutex::new(Default::default())));
    (store, service, session_id)
}

async fn append(
    store: &dyn SessionAppendStore,
    session_id: &SessionId,
    sequence: u64,
    payload: ItemPayload,
) {
    assert_eq!(
        store
            .append(
                session_id,
                sequence,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id: TurnId::new(),
                    sequence,
                    created_at_ms: sequence,
                    payload,
                },
            )
            .await
            .unwrap(),
        AppendOutcome::Appended
    );
}

fn item_events(events: &[ClientEvent]) -> Vec<&fabric::protocol::client::ItemEvent> {
    events
        .iter()
        .map(|event| match event {
            ClientEvent::Item(item) => item,
            other => panic!("unexpected daemon protocol event: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn daemon_protocol_projects_exactly_one_terminal_for_every_durable_item() {
    let (store, service, session_id) = fixture().await;
    append(
        store.as_ref(),
        &session_id,
        1,
        ItemPayload::AssistantMessage {
            content: "ok".into(),
        },
    )
    .await;
    append(
        store.as_ref(),
        &session_id,
        2,
        ItemPayload::ToolResult {
            call_id: "call-1".into(),
            content: "tool failed".into(),
            is_error: true,
            permit_id: None,
            audit_id: None,
        },
    )
    .await;

    let events = service
        .protocol_events_after(&session_id, &EventCursor::origin())
        .await
        .unwrap();
    let items = item_events(&events);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].phase, ItemPhase::Completed);
    assert_eq!(items[1].phase, ItemPhase::Failed);
    assert_eq!(items[1].error.as_deref(), Some("tool failed"));
    assert!(items.iter().all(|item| item.delta.is_none()));
    assert_eq!(
        items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        2
    );
}

#[tokio::test]
async fn daemon_cursor_reconnect_has_no_missing_or_duplicate_item_events() {
    let (store, service, session_id) = fixture().await;
    for sequence in 1..=4 {
        append(
            store.as_ref(),
            &session_id,
            sequence,
            ItemPayload::AssistantMessage {
                content: format!("item-{sequence}"),
            },
        )
        .await;
    }

    let initial = service
        .protocol_events_after(&session_id, &EventCursor::origin())
        .await
        .unwrap();
    let initial_items = item_events(&initial);
    let acknowledged = initial_items[1].cursor.clone();
    let before_disconnect = &initial_items[..2];

    let replay = service
        .protocol_events_after(&session_id, &acknowledged)
        .await
        .unwrap();
    let replay_items = item_events(&replay);
    let sequences = before_disconnect
        .iter()
        .chain(replay_items.iter())
        .map(|item| item.cursor.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
    assert_eq!(sequences.iter().copied().collect::<HashSet<_>>().len(), 4);

    let snapshot = service.protocol_snapshot(&session_id).await.unwrap();
    assert_eq!(snapshot.cursor.sequence, 4);
    assert_eq!(
        snapshot.cursor.event_id,
        replay_items.last().unwrap().cursor.event_id
    );

    let forged = EventCursor {
        sequence: 2,
        event_id: Some("different-event".into()),
    };
    assert!(service
        .protocol_events_after(&session_id, &forged)
        .await
        .is_err());
}

#[tokio::test]
async fn live_and_durable_item_phases_share_one_reconnect_cursor() {
    let temp = tempfile::tempdir().unwrap();
    let canonical_path = temp.path().join("sessions.db");
    let journal_path = temp.path().join("protocol.db");
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(&canonical_path).unwrap());
    let session_id = SessionId("live-reconnect".into());
    store
        .create(SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: session_id.clone(),
            parent: None,
            created_at_ms: 1,
            status: SessionStatus::Active,
        })
        .await
        .unwrap();
    let service = SessionService::with_protocol_journal(
        store.clone(),
        Arc::new(Mutex::new(Default::default())),
        &journal_path,
    )
    .unwrap();
    let turn_id = TurnId::new();
    let logical_id = format!("turn:{}:assistant", turn_id.0);
    let started = service
        .append_protocol_item_event(
            &session_id,
            logical_id.clone(),
            ItemPhase::Started,
            None,
            None,
            None,
            Some(format!("{logical_id}:assistant-started")),
        )
        .await
        .unwrap();
    service
        .append_protocol_item_event(
            &session_id,
            logical_id.clone(),
            ItemPhase::Streaming,
            Some("partial".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    store
        .append(
            &session_id,
            1,
            ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence: 1,
                created_at_ms: 2,
                payload: ItemPayload::AssistantMessage {
                    content: "complete".into(),
                },
            },
        )
        .await
        .unwrap();
    let started_cursor = match started {
        ClientEvent::Item(item) => item.cursor,
        _ => unreachable!(),
    };
    drop(service);
    let reopened = SessionService::with_protocol_journal(
        store,
        Arc::new(Mutex::new(Default::default())),
        &journal_path,
    )
    .unwrap();
    let replay = reopened
        .protocol_events_after(&session_id, &started_cursor)
        .await
        .unwrap();
    let phases = item_events(&replay)
        .into_iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(phases, vec![ItemPhase::Streaming, ItemPhase::Completed]);
    let cursors = item_events(&replay)
        .into_iter()
        .map(|event| event.cursor.sequence)
        .collect::<Vec<_>>();
    assert_eq!(cursors, vec![2, 3]);
    assert!(item_events(&replay)
        .iter()
        .all(|event| event.item_id == logical_id));
}
