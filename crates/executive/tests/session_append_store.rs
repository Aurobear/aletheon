use std::sync::Arc;

use executive::runtime::events::{DefaultEventProjectionSet, SqliteEventSpine};
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
    let store = executive::testing::turn_coordinator::compose_session_store(
        Arc::new(CanonicalSessionStore::open(&path).unwrap()),
        Arc::new(SqliteEventSpine::open(dir.path().join("events.db")).unwrap()),
        Arc::new(DefaultEventProjectionSet::in_memory()),
    );
    store.create(session("s1", None)).await.unwrap();
    let first = item(
        "s1",
        turn,
        1,
        ItemPayload::UserMessage {
            content: "hello".into(),
            execution_target: fabric::ExecutionTargetSelection::default(),
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
                    content: "other".into(),
                    execution_target: fabric::ExecutionTargetSelection::default(),
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
    let store = executive::testing::turn_coordinator::compose_in_memory_session_store(Arc::new(
        CanonicalSessionStore::open(":memory:").unwrap(),
    ));
    let parent = SessionId("parent".into());
    let turn = TurnId::new();
    store.create(session("parent", None)).await.unwrap();
    let original = item(
        "parent",
        turn,
        1,
        ItemPayload::UserMessage {
            content: "hello".into(),
            execution_target: fabric::ExecutionTargetSelection::default(),
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
                execution_target: fabric::ExecutionTargetSelection::default(),
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

#[tokio::test]
async fn different_sessions_append_concurrently_with_contiguous_per_session_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let store = executive::testing::turn_coordinator::compose_session_store(
        Arc::new(CanonicalSessionStore::open(dir.path().join("sessions.db")).unwrap()),
        Arc::new(SqliteEventSpine::open(dir.path().join("events.db")).unwrap()),
        Arc::new(DefaultEventProjectionSet::in_memory()),
    );
    let sessions = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for id in sessions {
        store.create(session(id, None)).await.unwrap();
    }

    // Concurrently append 10 items per session; each session's sequences must
    // be contiguous and conflict-free even though appends interleave.
    let store = Arc::new(store);
    let mut handles = Vec::new();
    for session_id in sessions {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let sid = SessionId(session_id.into());
            let turn = TurnId::new();
            for sequence in 1..=10u64 {
                store
                    .append(
                        &sid,
                        sequence,
                        item(
                            session_id,
                            turn,
                            sequence,
                            ItemPayload::UserMessage {
                                content: format!("{session_id}-{sequence}"),
                                execution_target: fabric::ExecutionTargetSelection::default(),
                            },
                        ),
                    )
                    .await
                    .expect("append must not conflict within one session");
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    for session_id in sessions {
        let items = store
            .load_items(&SessionId(session_id.into()), None)
            .await
            .unwrap();
        assert_eq!(
            items.len(),
            10,
            "session {session_id} must hold all 10 items"
        );
        let sequences: Vec<u64> = items.iter().map(|item| item.sequence).collect();
        assert_eq!(
            sequences,
            (1..=10).collect::<Vec<u64>>(),
            "contiguous sequences for {session_id}"
        );
    }
}

#[tokio::test]
async fn same_session_concurrent_appends_retry_to_one_contiguous_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(executive::testing::turn_coordinator::compose_session_store(
        Arc::new(CanonicalSessionStore::open(dir.path().join("sessions.db")).unwrap()),
        Arc::new(SqliteEventSpine::open(dir.path().join("events.db")).unwrap()),
        Arc::new(DefaultEventProjectionSet::in_memory()),
    ));
    let session_id = SessionId("contended".into());
    store.create(session("contended", None)).await.unwrap();
    let baseline = executive::runtime::session::event_sourced_store::session_append_metrics();
    let barrier = Arc::new(tokio::sync::Barrier::new(32));
    let turn = TurnId::new();
    let mut tasks = Vec::new();
    for sequence in 1..=32u64 {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .append(
                    &SessionId("contended".into()),
                    sequence,
                    item(
                        "contended",
                        turn,
                        sequence,
                        ItemPayload::SystemNotice {
                            content: format!("concurrent-{sequence}"),
                        },
                    ),
                )
                .await
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap(), AppendOutcome::Appended);
    }

    let items = store.load_items(&session_id, None).await.unwrap();
    assert_eq!(items.len(), 32);
    assert_eq!(
        items.iter().map(|item| item.sequence).collect::<Vec<_>>(),
        (1..=32).collect::<Vec<_>>()
    );
    let metrics = executive::runtime::session::event_sourced_store::session_append_metrics();
    assert!(metrics.sequence_retries > baseline.sequence_retries);
}

#[tokio::test]
async fn append_admission_uses_the_head_not_a_full_history_scan() {
    // The production append path now admits through the durable per-session
    // `next_sequence` head and an O(1) `item_by_id` idempotency lookup, not a
    // full-history `load_items` scan. This test asserts head-consistent
    // admission: a stale expected sequence is rejected once the head advances.
    let dir = tempfile::tempdir().unwrap();
    let store = executive::testing::turn_coordinator::compose_session_store(
        Arc::new(CanonicalSessionStore::open(dir.path().join("sessions.db")).unwrap()),
        Arc::new(SqliteEventSpine::open(dir.path().join("events.db")).unwrap()),
        Arc::new(DefaultEventProjectionSet::in_memory()),
    );
    store.create(session("head", None)).await.unwrap();
    let turn = TurnId::new();
    for sequence in 1..=5u64 {
        store
            .append(
                &SessionId("head".into()),
                sequence,
                item(
                    "head",
                    turn,
                    sequence,
                    ItemPayload::UserMessage {
                        content: format!("head-{sequence}"),
                        execution_target: fabric::ExecutionTargetSelection::default(),
                    },
                ),
            )
            .await
            .unwrap();
    }
    // A stale expected sequence must now be rejected (the head advanced past 5).
    let stale = store
        .append(
            &SessionId("head".into()),
            5,
            item(
                "head",
                turn,
                5,
                ItemPayload::UserMessage {
                    content: "stale".into(),
                    execution_target: fabric::ExecutionTargetSelection::default(),
                },
            ),
        )
        .await;
    assert!(stale.is_err(), "stale expected sequence must be rejected");
}
