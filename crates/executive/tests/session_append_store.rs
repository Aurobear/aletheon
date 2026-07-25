use executive::runtime::session::canonical_store::{project_messages, CanonicalSessionStore};
use fabric::*;

fn session(id: &str, parent: Option<SessionFork>) -> SessionRecord {
    SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: SessionId(id.into()),
        parent,
        created_at_ms: 1,
        status: SessionStatus::Active,
    }
}

fn item(session: &str, turn: TurnId, sequence: u64, payload: ItemPayload) -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId::new(),
        session_id: SessionId(session.into()),
        turn_id: turn,
        sequence,
        created_at_ms: sequence,
        payload,
    }
}

#[tokio::test]
async fn append_is_transactional_idempotent_and_restart_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let session_id = SessionId("s1".into());
    let turn = TurnId::new();
    let store = CanonicalSessionStore::open(&path).unwrap();
    store.create(session("s1", None)).await.unwrap();
    let first = item(
        "s1",
        turn,
        1,
        ItemPayload::UserMessage {
            content: "hello".into(),
        },
    );
    assert_eq!(
        store.append(&session_id, 1, first.clone()).await.unwrap(),
        AppendOutcome::Appended
    );
    assert_eq!(
        store.append(&session_id, 1, first).await.unwrap(),
        AppendOutcome::AlreadyPresent
    );
    assert!(store
        .append(
            &session_id,
            1,
            item(
                "s1",
                turn,
                1,
                ItemPayload::UserMessage {
                    content: "other".into()
                }
            )
        )
        .await
        .is_err());
    assert!(store
        .append(
            &session_id,
            3,
            item(
                "s1",
                turn,
                3,
                ItemPayload::AssistantMessage {
                    content: "gap".into()
                }
            )
        )
        .await
        .is_err());
    drop(store);

    let reopened = CanonicalSessionStore::open(&path).unwrap();
    assert_eq!(
        reopened.load_items(&session_id, None).await.unwrap().len(),
        1
    );
    assert!(reopened.load_session(&session_id).await.unwrap().is_some());
}

#[tokio::test]
async fn fork_copies_bounded_history_with_new_item_identity() {
    let store = CanonicalSessionStore::open(":memory:").unwrap();
    let parent = SessionId("parent".into());
    let turn = TurnId::new();
    store.create(session("parent", None)).await.unwrap();
    let original = item(
        "parent",
        turn,
        1,
        ItemPayload::UserMessage {
            content: "hello".into(),
        },
    );
    store.append(&parent, 1, original.clone()).await.unwrap();
    let child_record = session(
        "child",
        Some(SessionFork {
            session_id: parent.clone(),
            through_sequence: 1,
        }),
    );
    store.fork(&parent, 1, child_record).await.unwrap();
    let copied = store
        .load_items(&SessionId("child".into()), None)
        .await
        .unwrap();
    assert_eq!(copied.len(), 1);
    assert_ne!(copied[0].id, original.id);
    assert_eq!(copied[0].turn_id, original.turn_id);
    assert_eq!(copied[0].payload, original.payload);
}

#[test]
fn projection_is_deterministic_ordered_and_correlated() {
    let turn = TurnId::new();
    let items = vec![
        item(
            "s",
            turn,
            1,
            ItemPayload::SystemNotice {
                content: "system".into(),
            },
        ),
        item(
            "s",
            turn,
            2,
            ItemPayload::UserMessage {
                content: "user".into(),
            },
        ),
        item(
            "s",
            turn,
            3,
            ItemPayload::ToolCall {
                call_id: "call".into(),
                name: "tool".into(),
                input: serde_json::json!({"x":1}),
            },
        ),
        item(
            "s",
            turn,
            4,
            ItemPayload::ToolResult {
                call_id: "call".into(),
                content: "result".into(),
                is_error: false,
                permit_id: None,
                audit_id: None,
            },
        ),
        item(
            "s",
            turn,
            5,
            ItemPayload::AssistantMessage {
                content: "answer".into(),
            },
        ),
    ];
    let a = serde_json::to_vec(&project_messages(&items).unwrap()).unwrap();
    let b = serde_json::to_vec(&project_messages(&items).unwrap()).unwrap();
    assert_eq!(a, b);
    let mut invalid = items.clone();
    invalid.swap(2, 3);
    assert!(project_messages(&invalid).is_err());
    let text = String::from_utf8(a).unwrap();
    assert!(text.contains("\"id\":\"call\""));
    assert!(text.contains("\"tool_use_id\":\"call\""));
}
