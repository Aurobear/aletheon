use std::collections::BTreeMap;
use std::path::Path;

use fabric::protocol::client::{ApprovalEvent, EventCursor, ItemEvent, ItemPhase, UiSnapshot};
use fabric::{
    ApprovalCategory, ApprovalId, ApprovalRisk, ApprovalSnapshot, ApprovalStatus, ApprovalSubject,
    GoalId, ItemId, ItemPayload, ItemRecord, PrincipalId, SessionId, TurnId,
    SESSION_SCHEMA_VERSION,
};
use interact::tui::reducer::{reduce, snapshot_view, UiAction};
use interact::tui::state::AppState;

fn item(sequence: u64, payload: ItemPayload) -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(uuid::Uuid::from_u128(sequence as u128 + 10)),
        session_id: SessionId("snapshot-session".into()),
        turn_id: TurnId(uuid::Uuid::from_u128(2)),
        sequence,
        created_at_ms: sequence,
        payload,
    }
}

fn approval(status: ApprovalStatus, version: u64) -> ApprovalSnapshot {
    let subject = ApprovalSubject {
        category: ApprovalCategory::ApplyCode,
        goal_id: GoalId(1),
        attempt_id: None,
        job_id: None,
        attributes: BTreeMap::new(),
        allowed_scope: vec![],
        apply_target: None,
    };
    ApprovalSnapshot {
        id: ApprovalId(uuid::Uuid::from_u128(77)),
        goal_id: GoalId(1),
        attempt_id: None,
        job_id: None,
        owner_id: PrincipalId("owner".into()),
        category: ApprovalCategory::ApplyCode,
        risk: ApprovalRisk::High,
        subject_hash: subject.subject_hash().unwrap(),
        subject,
        summary: "apply verified patch".into(),
        artifacts: vec![],
        created_at_ms: 1,
        expires_at_ms: 100,
        status,
        version,
        resolution: None,
    }
}

fn assert_snapshot(name: &str, state: &AppState) {
    let rendered = serde_json::to_string_pretty(&snapshot_view(state)).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    if std::env::var_os("UPDATE_TUI_SNAPSHOTS").is_some() {
        std::fs::write(&path, &rendered).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), rendered);
}

#[test]
fn item_lifecycle_snapshot_covers_stream_terminal_tool_collapse_and_failure() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("snapshot-session".into()),
            cursor: EventCursor::origin(),
            provider: Some("openai".into()),
            model: Some("gpt-5".into()),
            items: vec![],
            approvals: vec![],
            agents: vec![],
        }),
    );
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 1,
                event_id: None,
            },
            item_id: "assistant-stream".into(),
            phase: ItemPhase::Streaming,
            delta: Some("hello".into()),
            item: None,
            error: None,
        }),
    );
    let tool = item(
        2,
        ItemPayload::ToolResult {
            call_id: "call-1".into(),
            content: "42 lines".into(),
            is_error: false,
            permit_id: None,
            audit_id: None,
        },
    );
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 2,
                event_id: None,
            },
            item_id: tool.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(tool),
            error: None,
        }),
    );
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 3,
                event_id: None,
            },
            item_id: "failed".into(),
            phase: ItemPhase::Failed,
            delta: None,
            item: None,
            error: Some("tool denied".into()),
        }),
    );
    assert_snapshot("item_lifecycle.snap", &state);
}

#[test]
fn approval_lifecycle_snapshot_covers_pending_and_failure_resolution() {
    let mut state = AppState::default();
    let pending = approval(ApprovalStatus::Pending, 1);
    reduce(
        &mut state,
        UiAction::Approval(ApprovalEvent {
            cursor: EventCursor {
                sequence: 1,
                event_id: None,
            },
            approval: pending,
        }),
    );
    let mut rejected = approval(ApprovalStatus::Rejected, 2);
    rejected.resolution = Some(fabric::ApprovalResolution::rejected(
        PrincipalId("owner".into()),
        "tui",
        2,
        Some("policy denied".into()),
    ));
    reduce(
        &mut state,
        UiAction::Approval(ApprovalEvent {
            cursor: EventCursor {
                sequence: 2,
                event_id: None,
            },
            approval: rejected,
        }),
    );
    assert_snapshot("approval_lifecycle.snap", &state);
}

#[test]
fn reconnect_resume_snapshot_has_no_lost_or_duplicate_items() {
    let first = item(
        1,
        ItemPayload::AssistantMessage {
            content: "first".into(),
        },
    );
    let second = item(
        2,
        ItemPayload::AssistantMessage {
            content: "second".into(),
        },
    );
    let mut state = AppState::default();
    reduce(
        &mut state,
        UiAction::Snapshot(UiSnapshot {
            session_id: SessionId("snapshot-session".into()),
            cursor: EventCursor {
                sequence: 10,
                event_id: Some("e10".into()),
            },
            provider: Some("anthropic".into()),
            model: Some("sonnet".into()),
            items: vec![first, second.clone()],
            approvals: vec![],
            agents: vec![],
        }),
    );
    reduce(
        &mut state,
        UiAction::Item(ItemEvent {
            cursor: EventCursor {
                sequence: 10,
                event_id: Some("e10".into()),
            },
            item_id: second.id.0.to_string(),
            phase: ItemPhase::Completed,
            delta: None,
            item: Some(second),
            error: None,
        }),
    );
    reduce(
        &mut state,
        UiAction::Reconnected(EventCursor {
            sequence: 9,
            event_id: None,
        }),
    );
    assert_snapshot("reconnect_resume.snap", &state);
}
