use std::{collections::HashSet, sync::Arc};

use executive::{
    application::session_service::SessionService,
    runtime::session::canonical_store::CanonicalSessionStore,
};
use fabric::{
    protocol::client::{ClientEvent, EventCursor, ItemPhase},
    AppendOutcome, ItemId, ItemPayload, ItemRecord, SessionAppendStore, SessionId, SessionRecord,
    SessionStatus, TurnId, SESSION_READ_MODEL_SCHEMA_VERSION, SESSION_SCHEMA_VERSION,
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

async fn append_record(store: &dyn SessionAppendStore, item: ItemRecord) -> AppendOutcome {
    store
        .append(&item.session_id.clone(), item.sequence, item)
        .await
        .unwrap()
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

/// A-SESSION-001: every persisted prefix can be reconstructed after losing
/// all process-local state and the derived protocol journal.
#[tokio::test]
async fn a_session_001_interruption_at_each_item_boundary_replays_the_same_task_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let canonical_path = temp.path().join("sessions.db");
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(&canonical_path).unwrap());
    let session_id = SessionId("a-session-001".into());
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
    let turn_id = TurnId::new();
    let payloads = [
        ItemPayload::UserMessage {
            content: "inspect the workspace".into(),
        },
        ItemPayload::ToolCall {
            call_id: "read-1".into(),
            name: "file_read".into(),
            input: serde_json::json!({"path":"README.md"}),
        },
        ItemPayload::ToolResult {
            call_id: "read-1".into(),
            content: "contents".into(),
            is_error: false,
            permit_id: None,
            audit_id: None,
        },
        ItemPayload::AssistantMessage {
            content: "done".into(),
        },
        ItemPayload::InferenceReceipt {
            receipt: fabric::types::inference_receipt::InferenceTerminalReceipt {
                schema_version:
                    fabric::types::inference_receipt::INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
                inference_id: "inference-1".into(),
                operation_id: "operation-1".into(),
                provider_id: "configured-provider".into(),
                model_id: "effective-model".into(),
                system_prefix_digest: "system-digest".into(),
                tool_schema_digest: "tool-digest".into(),
                status: fabric::types::inference_receipt::InferenceTerminalStatus::Succeeded,
                usage: fabric::InferenceUsage::reported(100, 20, Some(75), Some(25), Some(0)),
                failure_kind: None,
                prefix_shape_digest: None,
                local_cache_miss_reason: None,
            },
        },
    ];

    for (index, payload) in payloads.into_iter().enumerate() {
        let sequence = index as u64 + 1;
        assert_eq!(
            append_record(
                store.as_ref(),
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id,
                    sequence,
                    created_at_ms: sequence,
                    payload,
                },
            )
            .await,
            AppendOutcome::Appended
        );
        let before = SessionService::with_protocol_journal(
            store.clone(),
            Arc::new(Mutex::new(Default::default())),
            temp.path().join(format!("before-{sequence}.db")),
        )
        .unwrap()
        .protocol_read_snapshot(&session_id)
        .await
        .unwrap();
        let replayed_store: Arc<dyn SessionAppendStore> =
            Arc::new(CanonicalSessionStore::open(&canonical_path).unwrap());
        let after = SessionService::with_protocol_journal(
            replayed_store,
            Arc::new(Mutex::new(Default::default())),
            temp.path().join(format!("after-{sequence}.db")),
        )
        .unwrap()
        .protocol_read_snapshot(&session_id)
        .await
        .unwrap();
        assert_eq!(before.schema_version, SESSION_READ_MODEL_SCHEMA_VERSION);
        assert_eq!(before.tasks, after.tasks);
        assert_eq!(before.activities, after.activities);
        if sequence == 5 {
            let facts = after.tasks[0].runtime_facts.as_ref().unwrap();
            assert_eq!(
                facts.effective_provider.as_deref(),
                Some("configured-provider")
            );
            assert_eq!(facts.effective_model.as_deref(), Some("effective-model"));
            assert_eq!(facts.active_context_occupancy_tokens, None);
            assert_eq!(facts.cumulative_usage.total_input_tokens, Some(100));
            assert_eq!(facts.cumulative_usage.cache_read_tokens, Some(25));
            assert_eq!(facts.inference_rounds, 1);
        }
    }
    let sessions = SessionService::new(store, Arc::new(Mutex::new(Default::default())))
        .protocol_session_list()
        .await
        .unwrap();
    assert_eq!(sessions.schema_version, SESSION_READ_MODEL_SCHEMA_VERSION);
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].id, session_id);
}

/// A-SESSION-002: an idempotent item retry and an idempotent protocol event
/// retry each retain a single durable projection entry.
#[tokio::test]
async fn a_session_002_duplicate_item_and_event_keys_do_not_duplicate_task_or_activity_projection()
{
    let (store, service, session_id) = fixture().await;
    let turn_id = TurnId::new();
    let item = ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId::new(),
        session_id: session_id.clone(),
        turn_id,
        sequence: 1,
        created_at_ms: 1,
        payload: ItemPayload::ToolCall {
            call_id: "call-deduped".into(),
            name: "apply_patch".into(),
            input: serde_json::json!({"patch":"one-authoritative-effect"}),
        },
    };
    assert_eq!(
        append_record(store.as_ref(), item.clone()).await,
        AppendOutcome::Appended
    );
    assert_eq!(
        append_record(store.as_ref(), item).await,
        AppendOutcome::AlreadyPresent
    );

    let key = format!("tool:{}:call-deduped:tool-started", turn_id.0);
    let first = service
        .append_protocol_item_event(
            &session_id,
            format!("tool:{}:call-deduped", turn_id.0),
            ItemPhase::Started,
            None,
            None,
            None,
            Some(key.clone()),
        )
        .await
        .unwrap();
    let duplicate = service
        .append_protocol_item_event(
            &session_id,
            format!("tool:{}:call-deduped", turn_id.0),
            ItemPhase::Started,
            None,
            None,
            None,
            Some(key),
        )
        .await
        .unwrap();
    assert_eq!(first, duplicate);

    let snapshot = service.protocol_read_snapshot(&session_id).await.unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].steps.len(), 1);
    assert_eq!(snapshot.activities.len(), 1);
    let page = service
        .protocol_event_page(&session_id, &EventCursor::origin())
        .await
        .unwrap();
    assert_eq!(page.schema_version, SESSION_READ_MODEL_SCHEMA_VERSION);
    assert_eq!(page.next.sequence, page.events.len() as u64);
    assert_eq!(
        item_events(&page.events)
            .iter()
            .filter(|event| event.phase == ItemPhase::Started)
            .count(),
        1
    );
}
