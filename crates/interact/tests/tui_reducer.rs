use ::contracts::protocol::client::{
    ActivityKind, ActivitySnapshot, ActivityState, ClientEvent, EventCursor, ItemEvent, ItemPhase,
    SessionEventPage, SessionReadSnapshot, TaskPhase, TaskSnapshot, UiSnapshot,
};
use ::contracts::{
    EvaluationContractId, EvaluationDecision, EvaluationReceiptId, EvaluationReceiptRef, ItemId,
    ItemPayload, ItemRecord, SessionId, SessionRecord, SessionStatus, TurnId,
    SESSION_READ_MODEL_SCHEMA_VERSION, SESSION_SCHEMA_VERSION,
};
use interact::tui::reducer::{
    format_evaluation_receipt_ref, reduce, LiveActivityEvent, UiAction, UiEffect,
};
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

fn user_target(
    sequence: u64,
    turn: u128,
    execution_target: ::contracts::ExecutionTargetSelection,
) -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(uuid::Uuid::from_u128(sequence as u128 + 1_000)),
        session_id: SessionId("session-1".into()),
        turn_id: TurnId(uuid::Uuid::from_u128(turn)),
        sequence,
        created_at_ms: sequence,
        payload: ItemPayload::UserMessage {
            content: format!("turn-{turn}"),
            execution_target,
        },
    }
}

fn evaluation(sequence: u64) -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(uuid::Uuid::from_u128(sequence as u128 + 100)),
        session_id: SessionId("session-1".into()),
        turn_id: TurnId(uuid::Uuid::from_u128(1)),
        sequence,
        created_at_ms: sequence,
        payload: ItemPayload::EvaluationReceiptRef {
            receipt: EvaluationReceiptRef {
                schema_version: 1,
                receipt_id: EvaluationReceiptId(uuid::Uuid::from_u128(2)),
                contract_id: EvaluationContractId(uuid::Uuid::from_u128(3)),
                subject_kind: "turn".into(),
                subject_id: "turn-1".into(),
                decision: EvaluationDecision::ObservedFail,
                weighted_total_millis: Some(82_400),
                evidence_coverage_millis: 800,
                confidence_millis: 1_000,
                failed_gates: vec!["tests_passed".into()],
                created_at_ms: 1,
            },
        },
    }
}

fn read_snapshot(sequence: u64, items: Vec<ItemRecord>) -> SessionReadSnapshot {
    let session_id = SessionId("session-1".into());
    SessionReadSnapshot {
        schema_version: SESSION_READ_MODEL_SCHEMA_VERSION,
        session: SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: session_id.clone(),
            parent: None,
            created_at_ms: 1,
            status: SessionStatus::Active,
        },
        through: EventCursor {
            sequence,
            event_id: (sequence > 0).then(|| format!("event-{sequence}")),
        },
        items,
        tasks: vec![TaskSnapshot {
            task_id: "session:session-1:task".into(),
            session_id,
            goal: Some("authoritative goal".into()),
            phase: TaskPhase::Active,
            plan_revision: None,
            steps: vec![],
            active_turn_id: None,
            active_runtime_children: vec![],
            active_commands: vec![],
            pending_approvals: vec![],
            budget: None,
            checkpoint_head: None,
            checkpoint_review: None,
            settlement: None,
            review_findings: vec![],
            runtime_facts: None,
        }],
        activities: vec![],
    }
}

#[test]
fn live_tool_overlay_is_visible_then_atomically_replaced_by_durable_activity() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::LiveActivity(LiveActivityEvent::ToolStarted {
            call_id: "call-1".into(),
            tool: "file_read".into(),
            args: serde_json::json!({"path":"src/lib.rs"}),
            observed_at: 10,
        }),
    );
    assert!(state.activities.iter().any(|activity| {
        activity.activity_id.ends_with(":tool:call-1") && activity.state == ActivityState::Running
    }));

    let mut snapshot = read_snapshot(1, Vec::new());
    snapshot.activities.push(ActivitySnapshot {
        activity_id: "tool:00000000-0000-0000-0000-000000000001:call-1".into(),
        task_id: "session:session-1:task".into(),
        turn_id: TurnId(uuid::Uuid::from_u128(1)),
        parent_activity_id: None,
        kind: ActivityKind::Tool,
        label: "file_read".into(),
        state: ActivityState::Completed,
        started_at: 10,
        updated_at: 20,
        progress: None,
        artifact_refs: Vec::new(),
        receipt_ref: Some("item:result-1".into()),
    });
    reduce(&mut state, UiAction::ReadSnapshot(snapshot));

    assert_eq!(state.activities.len(), 1);
    assert_eq!(
        state.activities[0].activity_id,
        "tool:00000000-0000-0000-0000-000000000001:call-1"
    );
    assert_eq!(state.activities[0].state, ActivityState::Completed);
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
        vec![
            UiEffect::ReloadSnapshot(SessionId("session-1".into())),
            UiEffect::SubscribeAfter(EventCursor {
                sequence: 11,
                event_id: Some("e11".into())
            })
        ]
    );
}

#[test]
fn general_robot_general_start_facts_project_only_the_latest_turn_target() {
    let robot = ::contracts::ExecutionTargetSelection::robot(
        "robot-1",
        ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
        ::contracts::ExecutionTargetSource::UserCommand,
    )
    .unwrap();
    let final_general = ::contracts::ExecutionTargetSelection::general(
        ::contracts::ExecutionTargetSource::UserCommand,
    );
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("session-1".into()),
            cursor: EventCursor {
                sequence: 3,
                event_id: Some("e3".into()),
            },
            provider: None,
            model: None,
            items: vec![
                user_target(1, 1, ::contracts::ExecutionTargetSelection::default()),
                user_target(2, 2, robot),
                user_target(3, 3, final_general.clone()),
            ],
            approvals: vec![],
            agents: vec![],
        }),
    );
    assert_eq!(state.execution_target, final_general);
    assert_eq!(state.items.len(), 3);
    assert_eq!(
        state
            .items
            .values()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn local_target_selection_survives_stale_projection_until_newer_start_is_durable() {
    let robot = ::contracts::ExecutionTargetSelection::robot(
        "robot-1",
        ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
        ::contracts::ExecutionTargetSource::UserCommand,
    )
    .unwrap();
    let explicit_general = ::contracts::ExecutionTargetSelection::general(
        ::contracts::ExecutionTargetSource::UserCommand,
    );
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("session-1".into()),
            cursor: EventCursor {
                sequence: 10,
                event_id: Some("e10".into()),
            },
            provider: None,
            model: None,
            items: vec![user_target(
                1,
                1,
                ::contracts::ExecutionTargetSelection::default(),
            )],
            approvals: vec![],
            agents: vec![],
        }),
    );

    state.select_execution_target_for_next_turn(robot.clone());
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("session-1".into()),
            cursor: EventCursor {
                sequence: 10,
                event_id: Some("e10".into()),
            },
            provider: None,
            model: None,
            items: vec![user_target(
                1,
                1,
                ::contracts::ExecutionTargetSelection::default(),
            )],
            approvals: vec![],
            agents: vec![],
        }),
    );
    assert_eq!(state.execution_target_for_submission(), &robot);
    assert!(state.has_pending_execution_target());

    let robot_start = user_target(11, 2, robot.clone());
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 11,
                event_id: Some("e11".into()),
            },
            item_id: robot_start.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(robot_start),
            error: None,
        }),
    );
    assert_eq!(state.execution_target, robot);
    assert!(!state.has_pending_execution_target());

    state.select_execution_target_for_next_turn(explicit_general.clone());
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("session-1".into()),
            cursor: EventCursor {
                sequence: 11,
                event_id: Some("e11".into()),
            },
            provider: None,
            model: None,
            items: vec![
                user_target(1, 1, ::contracts::ExecutionTargetSelection::default()),
                user_target(11, 2, robot),
            ],
            approvals: vec![],
            agents: vec![],
        }),
    );
    assert_eq!(state.execution_target_for_submission(), &explicit_general);
    assert!(state.has_pending_execution_target());

    let general_start = user_target(12, 3, explicit_general.clone());
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 12,
                event_id: Some("e12".into()),
            },
            item_id: general_start.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(general_start),
            error: None,
        }),
    );
    assert_eq!(state.execution_target, explicit_general);
    assert!(!state.has_pending_execution_target());
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

#[test]
fn evaluation_receipt_is_cached_and_rendered_as_separate_metrics() {
    let mut state = AppState::default();
    let record = evaluation(1);
    let receipt = match &record.payload {
        ItemPayload::EvaluationReceiptRef { receipt } => receipt.clone(),
        _ => unreachable!(),
    };

    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 1,
                event_id: None,
            },
            item_id: record.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(record),
            error: None,
        }),
    );

    assert_eq!(state.latest_evaluation, Some(receipt.clone()));
    assert_eq!(
        format_evaluation_receipt_ref(&receipt),
        "[evaluation] decision=observed_fail score=82.4 coverage=80.0% confidence=100.0% failed_gates=tests_passed"
    );
}

#[test]
fn a_session_003_local_state_loss_recovers_from_daemon_snapshot_and_ordered_pages() {
    let snapshot = read_snapshot(10, vec![completed(1, "durable")]);
    let mut retained = AppState::default();
    retained.model_name = "local-only-model-label".into();
    retained.items.insert(
        "local-draft".into(),
        interact::tui::state::UiItem::streaming("local-draft".into()),
    );
    let retained_effects = reduce(&mut retained, UiAction::ReadSnapshot(snapshot.clone()));

    let mut recovered = AppState::default();
    let recovered_effects = reduce(&mut recovered, UiAction::ReadSnapshot(snapshot));
    assert_eq!(retained_effects, recovered_effects);
    assert_eq!(retained.cursor, recovered.cursor);
    assert_eq!(retained.session_id, recovered.session_id);
    assert_eq!(retained.projected_session, recovered.projected_session);
    assert_eq!(retained.items.len(), recovered.items.len());
    assert_eq!(
        retained
            .items
            .values()
            .map(|item| (&item.id, &item.content))
            .collect::<Vec<_>>(),
        recovered
            .items
            .values()
            .map(|item| (&item.id, &item.content))
            .collect::<Vec<_>>()
    );
    assert_eq!(retained.tasks, recovered.tasks);
    assert_eq!(retained.activities, recovered.activities);
    assert!(!retained.items.contains_key("local-draft"));

    let next = completed(2, "tail");
    let page = SessionEventPage {
        schema_version: SESSION_READ_MODEL_SCHEMA_VERSION,
        session_id: SessionId("session-1".into()),
        after: recovered.cursor.clone(),
        next: EventCursor {
            sequence: 11,
            event_id: Some("event-11".into()),
        },
        events: vec![ClientEvent::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 11,
                event_id: Some("event-11".into()),
            },
            item_id: "turn:assistant".into(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(next),
            error: None,
        })],
    };
    assert_eq!(
        reduce(&mut recovered, UiAction::EventPage(page.clone())),
        vec![
            UiEffect::Render,
            UiEffect::SubscribeAfter(EventCursor {
                sequence: 11,
                event_id: Some("event-11".into())
            })
        ]
    );
    assert!(reduce(&mut recovered, UiAction::EventPage(page)).is_empty());

    let before = recovered.cursor.clone();
    let out_of_order = SessionEventPage {
        schema_version: SESSION_READ_MODEL_SCHEMA_VERSION,
        session_id: SessionId("session-1".into()),
        after: EventCursor::origin(),
        next: EventCursor {
            sequence: 12,
            event_id: Some("event-12".into()),
        },
        events: vec![ClientEvent::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 12,
                event_id: Some("event-12".into()),
            },
            item_id: "gap".into(),
            phase: ItemPhase::Started,
            delta: None,
            item: None,
            error: None,
        })],
    };
    let effects = reduce(&mut recovered, UiAction::EventPage(out_of_order));
    assert!(matches!(
        effects.as_slice(),
        [UiEffect::AnnounceError(_), UiEffect::ReloadSnapshot(_)]
    ));
    assert_eq!(recovered.cursor, before);
}

#[test]
fn live_assistant_text_is_visible_then_atomically_replaced_by_durable_item() {
    let mut state = AppState::default();
    state.session_id = Some("session-1".into());
    state.active_turn_id = Some(TurnId(uuid::Uuid::from_u128(1)));

    // Live streamed text appears on the canonical surface before durable commit.
    reduce(
        &mut state,
        UiAction::LiveAssistantText {
            text: "streaming answer…".into(),
            sequence: 7,
        },
    );
    let live = state
        .items
        .values()
        .find(|item| item.id.starts_with("live:") && item.id.ends_with(":assistant"))
        .expect("live assistant overlay must be visible on the canonical surface");
    assert_eq!(live.kind, "assistant");
    assert_eq!(live.content, "streaming answer…");

    // A durable assistant item at the same sequence commits: the overlay is
    // removed and the durable item wins, so no duplication can occur.
    let durable = completed(7, "committed answer");
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 7,
                event_id: None,
            },
            item_id: durable.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(durable),
            error: None,
        }),
    );
    assert!(
        !state
            .items
            .keys()
            .any(|id| id.starts_with("live:") && id.ends_with(":assistant")),
        "live overlay must be cleared once the durable item commits"
    );
    let committed = state
        .items
        .values()
        .find(|item| item.content == "committed answer")
        .expect("durable assistant item must be present");
    assert_eq!(committed.kind, "assistant");
}

#[test]
fn live_assistant_text_does_not_displace_an_already_committed_item() {
    let mut state = AppState::default();
    state.session_id = Some("session-1".into());
    state.active_turn_id = Some(TurnId(uuid::Uuid::from_u128(1)));
    let durable = completed(9, "committed");
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 9,
                event_id: None,
            },
            item_id: durable.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(durable),
            error: None,
        }),
    );

    // A late live delta at an older sequence must not create a duplicate.
    reduce(
        &mut state,
        UiAction::LiveAssistantText {
            text: "stale live text".into(),
            sequence: 9,
        },
    );
    assert!(
        !state
            .items
            .keys()
            .any(|id| id.starts_with("live:") && id.ends_with(":assistant")),
        "live overlay must be dropped when a durable assistant item already exists"
    );
}

#[test]
fn durable_assistant_clears_compatibility_overlay_with_local_turn_identity() {
    let mut state = AppState::default();
    state.session_id = Some("session-1".into());

    // The legacy stream does not carry a canonical turn ID, so the reducer
    // creates a local identity until the durable projection catches up.
    reduce(
        &mut state,
        UiAction::LiveAssistantText {
            text: "streamed answer".into(),
            sequence: 7,
        },
    );
    assert!(state
        .items
        .keys()
        .any(|id| id.starts_with("live:") && id.ends_with(":assistant")));

    // The durable record carries a different, authoritative turn ID. It must
    // still replace the sole live assistant overlay rather than render twice.
    let durable = completed(8, "streamed answer");
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 8,
                event_id: None,
            },
            item_id: durable.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(durable),
            error: None,
        }),
    );

    assert_eq!(
        state
            .items
            .values()
            .filter(|item| item.kind == "assistant")
            .count(),
        1
    );
    assert!(!state
        .items
        .keys()
        .any(|id| id.starts_with("live:") && id.ends_with(":assistant")));
}
