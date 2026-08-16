use application::session_input::*;
use contracts::{ConnectionId, PrincipalId, SchemaId, ThreadId};
use runtime::event_projection::CanonicalEventBus;
use runtime::{PromptKind, QueueOpResult};
use std::sync::Arc;
use uuid::Uuid;

fn principal(value: &str) -> PrincipalId {
    PrincipalId(value.into())
}

fn connection(value: u128) -> ConnectionId {
    ConnectionId(Uuid::from_u128(value))
}

fn thread(value: &str) -> ThreadId {
    ThreadId(value.into())
}

#[tokio::test]
async fn concurrent_enqueue_has_one_observable_total_order() {
    let coordinator = Arc::new(SessionInputCoordinator::in_memory());
    let first = coordinator.enqueue(
        principal("p"),
        connection(1),
        thread("t"),
        PromptKind::Prompt,
        "first".into(),
        "i1".into(),
    );
    let second = coordinator.enqueue(
        principal("p"),
        connection(2),
        thread("t"),
        PromptKind::Prompt,
        "second".into(),
        "i2".into(),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    let snapshot = coordinator
        .snapshot(&principal("p"), &thread("t"))
        .await
        .unwrap();

    assert_eq!(snapshot.pending.len(), 2);
    let mut positions = snapshot
        .queue_position
        .values()
        .copied()
        .collect::<Vec<_>>();
    positions.sort_unstable();
    assert_eq!(positions, [0, 1]);
    assert!(snapshot.queue_position.contains_key(&first.prompt_id));
    assert!(snapshot.queue_position.contains_key(&second.prompt_id));
}

#[tokio::test]
async fn edit_enforces_version_owner_and_last_editor() {
    let coordinator = SessionInputCoordinator::in_memory();
    let queued = coordinator
        .enqueue(
            principal("owner"),
            connection(1),
            thread("t"),
            PromptKind::Prompt,
            "old".into(),
            "idem".into(),
        )
        .await
        .unwrap();

    let stale = coordinator
        .edit(
            queued.prompt_id,
            9,
            (principal("owner"), connection(2)),
            "stale".into(),
        )
        .await
        .unwrap();
    assert!(matches!(stale, QueueOpResult::Conflict { .. }));
    let foreign = coordinator
        .edit(
            queued.prompt_id,
            0,
            (principal("other"), connection(2)),
            "foreign".into(),
        )
        .await
        .unwrap();
    assert!(matches!(foreign, QueueOpResult::Rejected { .. }));
    let updated = coordinator
        .edit(
            queued.prompt_id,
            0,
            (principal("owner"), connection(2)),
            "new".into(),
        )
        .await
        .unwrap();
    assert_eq!(updated, QueueOpResult::Ok { new_version: 1 });

    let stored = coordinator
        .queue_store()
        .get(queued.prompt_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.principal_id, principal("owner"));
    assert_eq!(stored.connection_id, connection(2));
    assert_eq!(stored.content, "new");
    assert_eq!(
        coordinator
            .metrics(&principal("owner"), &thread("t"))
            .await
            .unwrap()
            .prompt_edit_conflict_total,
        1
    );
}

#[tokio::test]
async fn running_edit_rejected_and_cancel_checks_version() {
    let coordinator = SessionInputCoordinator::in_memory();
    let queued = coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Prompt,
            "run".into(),
            "idem".into(),
        )
        .await
        .unwrap();
    let running = coordinator
        .take_next(&principal("p"), &thread("t"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(running.prompt_id, queued.prompt_id);
    assert!(matches!(
        coordinator
            .edit(
                running.prompt_id,
                running.version,
                (principal("p"), connection(2)),
                "edit".into()
            )
            .await
            .unwrap(),
        QueueOpResult::Rejected { .. }
    ));
    assert!(matches!(
        coordinator
            .cancel(running.prompt_id, 0, principal("p"))
            .await
            .unwrap(),
        QueueOpResult::Conflict { .. }
    ));
    assert_eq!(
        coordinator
            .cancel(running.prompt_id, running.version, principal("p"))
            .await
            .unwrap(),
        QueueOpResult::Ok {
            new_version: running.version + 1
        }
    );
}

#[tokio::test]
async fn interjections_are_bounded_fifo_independent_and_idempotent() {
    let coordinator = SessionInputCoordinator::in_memory();
    let oversized = format!("{}界", "a".repeat(runtime::MAX_INTERJECTION_BYTES - 1));
    coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Interjection,
            oversized,
            "i1".into(),
        )
        .await
        .unwrap();
    coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Interjection,
            "second".into(),
            "i2".into(),
        )
        .await
        .unwrap();

    let drained = coordinator
        .drain_interjections_at_safe_point(&principal("p"), &thread("t"), "turn-1")
        .await
        .unwrap();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].len(), runtime::MAX_INTERJECTION_BYTES - 1);
    assert_eq!(drained[1], "second");
    assert!(coordinator
        .drain_interjections_at_safe_point(&principal("p"), &thread("t"), "turn-1",)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        coordinator
            .metrics(&principal("p"), &thread("t"))
            .await
            .unwrap()
            .interjection_dropped_bytes_total,
        "界".len() as u64
    );
}

#[tokio::test]
async fn snapshots_are_partitioned_by_principal_and_thread() {
    let coordinator = SessionInputCoordinator::in_memory();
    for (p, t, text, idem) in [
        ("p1", "t1", "visible", "i1"),
        ("p1", "t2", "other-thread", "i2"),
        ("p2", "t1", "other-principal", "i3"),
    ] {
        coordinator
            .enqueue(
                principal(p),
                connection(1),
                thread(t),
                PromptKind::Prompt,
                text.into(),
                idem.into(),
            )
            .await
            .unwrap();
    }
    let snapshot = coordinator
        .snapshot(&principal("p1"), &thread("t1"))
        .await
        .unwrap();
    assert_eq!(snapshot.pending.len(), 1);
    assert_eq!(snapshot.pending[0].content, "visible");
}

#[tokio::test]
async fn idempotency_key_deduplicates_replay() {
    let coordinator = SessionInputCoordinator::in_memory();
    let first = coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Prompt,
            "first".into(),
            "same".into(),
        )
        .await
        .unwrap();
    let replay = coordinator
        .enqueue(
            principal("p"),
            connection(2),
            thread("t"),
            PromptKind::Prompt,
            "changed".into(),
            "same".into(),
        )
        .await
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        coordinator
            .snapshot(&principal("p"), &thread("t"))
            .await
            .unwrap()
            .pending
            .len(),
        1
    );
}

#[tokio::test]
async fn claimed_processor_runs_queued_prompts_in_order_then_releases() {
    let coordinator = SessionInputCoordinator::in_memory();
    for (content, idem) in [("first", "one"), ("second", "two")] {
        coordinator
            .enqueue(
                principal("p"),
                connection(1),
                thread("t"),
                PromptKind::Prompt,
                content.into(),
                idem.into(),
            )
            .await
            .unwrap();
    }

    assert!(
        coordinator
            .try_claim_processor(&principal("p"), &thread("t"))
            .await
    );
    assert!(
        !coordinator
            .try_claim_processor(&principal("p"), &thread("t"))
            .await
    );
    let first = coordinator
        .take_next_or_release(&principal("p"), &thread("t"))
        .await
        .unwrap()
        .unwrap();
    coordinator
        .mark_prompt_completed(first.prompt_id, "receipt-1")
        .await
        .unwrap();
    let second = coordinator
        .take_next_or_release(&principal("p"), &thread("t"))
        .await
        .unwrap()
        .unwrap();
    coordinator
        .mark_prompt_completed(second.prompt_id, "receipt-2")
        .await
        .unwrap();
    assert_eq!([first.content, second.content], ["first", "second"]);
    assert!(coordinator
        .take_next_or_release(&principal("p"), &thread("t"))
        .await
        .unwrap()
        .is_none());
    assert!(
        coordinator
            .try_claim_processor(&principal("p"), &thread("t"))
            .await
    );
}

#[tokio::test]
async fn queue_events_are_canonical_thread_partitioned_and_content_free() {
    let bus = Arc::new(CanonicalEventBus::new(8));
    let mut enqueued = bus.subscribe_channel(SchemaId::from(SchemaId::EVENT_PROMPT_ENQUEUED_V1));
    let mut edited = bus.subscribe_channel(SchemaId::from(SchemaId::EVENT_PROMPT_EDITED_V1));
    let mut cancelled = bus.subscribe_channel(SchemaId::from(SchemaId::EVENT_PROMPT_CANCELLED_V1));
    let mut consumed =
        bus.subscribe_channel(SchemaId::from(SchemaId::EVENT_INTERJECTION_CONSUMED_V1));
    let spine = Arc::new(
        adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").expect("event spine"),
    );
    let coordinator = SessionInputCoordinator::in_memory()
        .with_event_bus(bus)
        .with_event_spine(spine.clone());

    let prompt = coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Prompt,
            "secret prompt text".into(),
            "event-idem".into(),
        )
        .await
        .unwrap();
    let edited_result = coordinator
        .edit(
            prompt.prompt_id,
            prompt.version,
            (principal("p"), connection(2)),
            "new secret text".into(),
        )
        .await
        .unwrap();
    let edited_version = match edited_result {
        QueueOpResult::Ok { new_version } => new_version,
        other => panic!("unexpected edit result: {other:?}"),
    };
    coordinator
        .cancel(prompt.prompt_id, edited_version, principal("p"))
        .await
        .unwrap();
    coordinator
        .enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Interjection,
            "interjection secret".into(),
            "interjection-idem".into(),
        )
        .await
        .unwrap();
    coordinator
        .drain_interjections_at_safe_point(&principal("p"), &thread("t"), "receipt")
        .await
        .unwrap();

    for event in [
        enqueued.recv().await.unwrap(),
        edited.recv().await.unwrap(),
        cancelled.recv().await.unwrap(),
        consumed.recv().await.unwrap(),
    ] {
        assert_eq!(event.target.0, "thread:t");
        assert_eq!(event.namespace.0, "principal:p");
        assert_eq!(event.payload["thread_id"], "t");
        assert!(event.payload.get("content").is_none());
        assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    }
    assert_eq!(spine.metrics().accepted, 5);
}
