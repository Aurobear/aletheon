use fabric::protocol::client::{EventCursor, ItemEvent, ItemPhase, UiSnapshot};
use fabric::{ItemId, ItemPayload, ItemRecord, SessionId, TurnId, SESSION_SCHEMA_VERSION};
use interact::tui::reducer::{reduce, UiAction, UiEffect};
use interact::tui::state::AppState;

fn completed(sequence: u64, content: &str) -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(uuid::Uuid::from_u128(sequence as u128 + 1)),
        session_id: SessionId("session-1".into()),
        turn_id: TurnId(uuid::Uuid::from_u128(1)),
        sequence,
        created_at_ms: sequence,
        payload: ItemPayload::AssistantMessage {
            content: content.into(),
        },
    }
}

#[test]
fn snapshot_then_incremental_events_and_reconnect_are_deterministic_and_idempotent() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("session-1".into()),
            cursor: EventCursor {
                sequence: 10,
                event_id: Some("e10".into()),
            },
            provider: Some("anthropic".into()),
            model: Some("sonnet".into()),
            items: vec![completed(1, "done")],
            approvals: vec![],
            agents: vec![],
        }),
    );
    let item = completed(2, "second");
    let event = ItemEvent {
        cursor: EventCursor {
            sequence: 11,
            event_id: Some("e11".into()),
        },
        item_id: item.id.0.to_string(),
        phase: ItemPhase::Completed,
        delta: None,
        item: Some(item.clone()),
        error: None,
    };
    assert_eq!(
        reduce(&mut state, UiAction::Item(event.clone())),
        vec![UiEffect::Render]
    );
    assert!(reduce(&mut state, UiAction::Item(event)).is_empty());
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.provider_name.as_deref(), Some("anthropic"));
    assert_eq!(state.model_name, "sonnet");

    let effects = reduce(
        &mut state,
        UiAction::Reconnected(EventCursor {
            sequence: 9,
            event_id: None,
        }),
    );
    assert_eq!(
        effects,
        vec![UiEffect::SubscribeAfter(EventCursor {
            sequence: 11,
            event_id: Some("e11".into())
        })]
    );
}

#[test]
fn streaming_failure_is_a_pure_state_transition() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 1,
                event_id: None,
            },
            item_id: "stream-1".into(),
            phase: ItemPhase::Streaming,
            delta: Some("partial".into()),
            item: None,
            error: None,
        }),
    );
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 2,
                event_id: None,
            },
            item_id: "stream-1".into(),
            phase: ItemPhase::Failed,
            delta: None,
            item: None,
            error: Some("connection lost".into()),
        }),
    );
    assert_eq!(state.items["stream-1"].content, "connection lost");
}
