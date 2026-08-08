//! Pure protocol-to-view-state reducer.

use fabric::protocol::client::{
    AgentEvent, ApprovalEvent, EventCursor, ItemEvent, ItemPhase, SessionEventPage,
    SessionReadSnapshot, UiSnapshot,
};
use fabric::{EvaluationDecision, EvaluationReceiptRef, ItemPayload, ItemRecord};
use serde::Serialize;

use super::state::{AppState, UiItem, UiItemStatus};

#[derive(Debug, Clone)]
pub enum UiAction {
    Snapshot(UiSnapshot),
    ReadSnapshot(SessionReadSnapshot),
    EventPage(SessionEventPage),
    Item(ItemEvent),
    Approval(ApprovalEvent),
    Agent(AgentEvent),
    Reconnected(EventCursor),
    Failed(UiError),
}

#[derive(Debug, Clone)]
pub struct UiError {
    pub cursor: Option<EventCursor>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEffect {
    Render,
    SubscribeAfter(EventCursor),
    ReloadSnapshot(fabric::SessionId),
    AnnounceError(String),
}

pub fn reduce(state: &mut AppState, action: UiAction) -> Vec<UiEffect> {
    match action {
        UiAction::Snapshot(snapshot) => {
            state.cursor = snapshot.cursor;
            state.session_id = Some(snapshot.session_id.0);
            state.provider_name = snapshot.provider;
            state.model_name = snapshot.model.unwrap_or_else(|| "unknown".into());
            state.items.clear();
            state.latest_evaluation = None;
            for item in snapshot.items {
                upsert_completed(state, item);
            }
            state.approvals = snapshot
                .approvals
                .into_iter()
                .map(|approval| (approval.id.to_string(), approval))
                .collect();
            state.agents = snapshot
                .agents
                .into_iter()
                .map(|agent| (agent.handle.agent_id.0.to_string(), agent))
                .collect();
            state.last_error = None;
            vec![UiEffect::Render]
        }
        UiAction::ReadSnapshot(snapshot) => reduce_read_snapshot(state, snapshot),
        UiAction::EventPage(page) => reduce_event_page(state, page),
        UiAction::Item(event) => {
            if event.cursor.sequence <= state.cursor.sequence {
                return Vec::new();
            }
            apply_item_event(state, event);
            vec![UiEffect::Render]
        }
        UiAction::Approval(event) => {
            if advance(state, &event.cursor) {
                state
                    .approvals
                    .insert(event.approval.id.to_string(), event.approval);
                vec![UiEffect::Render]
            } else {
                Vec::new()
            }
        }
        UiAction::Agent(event) => {
            if advance(state, &event.cursor) {
                state
                    .agents
                    .insert(event.agent.handle.agent_id.0.to_string(), event.agent);
                vec![UiEffect::Render]
            } else {
                Vec::new()
            }
        }
        UiAction::Reconnected(cursor) => {
            if cursor.sequence > state.cursor.sequence {
                state.cursor = cursor;
            }
            vec![UiEffect::SubscribeAfter(state.cursor.clone())]
        }
        UiAction::Failed(error) => {
            if let Some(cursor) = error.cursor {
                if cursor.sequence > state.cursor.sequence {
                    state.cursor = cursor;
                }
            }
            state.last_error = Some(error.message.clone());
            vec![UiEffect::AnnounceError(error.message), UiEffect::Render]
        }
    }
}

fn reduce_read_snapshot(state: &mut AppState, snapshot: SessionReadSnapshot) -> Vec<UiEffect> {
    if snapshot.schema_version != fabric::SESSION_READ_MODEL_SCHEMA_VERSION {
        return vec![UiEffect::AnnounceError(format!(
            "unsupported Session read-model schema {}",
            snapshot.schema_version
        ))];
    }
    if snapshot
        .items
        .iter()
        .any(|item| item.session_id != snapshot.session.id)
        || snapshot
            .tasks
            .iter()
            .any(|task| task.session_id != snapshot.session.id)
        || snapshot.activities.iter().any(|activity| {
            !snapshot
                .tasks
                .iter()
                .any(|task| task.task_id == activity.task_id)
        })
    {
        return vec![UiEffect::AnnounceError(
            "Session read snapshot contains cross-session projection data".into(),
        )];
    }

    state.cursor = snapshot.through;
    state.session_id = Some(snapshot.session.id.0.clone());
    state.projected_session = Some(snapshot.session);
    state.items.clear();
    state.latest_evaluation = None;
    for item in snapshot.items {
        upsert_completed(state, item);
    }
    state.tasks = snapshot.tasks;
    state.activities = snapshot.activities;
    project_runtime_accounting(state);
    state.last_error = None;
    vec![
        UiEffect::Render,
        UiEffect::SubscribeAfter(state.cursor.clone()),
    ]
}

fn project_runtime_accounting(state: &mut AppState) {
    let facts = state
        .tasks
        .iter()
        .find(|task| {
            matches!(
                task.phase,
                fabric::TaskPhase::Active | fabric::TaskPhase::Interrupted
            )
        })
        .or_else(|| state.tasks.first())
        .and_then(|task| task.runtime_facts.as_ref());
    let Some(facts) = facts else {
        if !state.turn_active {
            state.context.used = 0;
            state.context.max = 0;
            state.total_tokens = 0;
            state.turn_input_tokens = 0;
            state.turn_output_tokens = 0;
        }
        return;
    };
    let projected_total = facts
        .cumulative_usage
        .total_input_tokens
        .unwrap_or(0)
        .saturating_add(facts.cumulative_usage.output_tokens.unwrap_or(0));
    if state.turn_active {
        // Snapshot polling can trail the live turn stream. Never let a stale
        // read projection roll back usage/context that the provider event has
        // already reported for the active turn.
        state.total_tokens = state.total_tokens.max(projected_total);
        if state.context.max == 0 {
            state.context.max = facts.context_capacity_tokens.unwrap_or(0);
        }
    } else {
        state.context.used = facts.active_context_occupancy_tokens.unwrap_or(0);
        state.context.max = facts.context_capacity_tokens.unwrap_or(0);
        state.total_tokens = projected_total;
    }
    // A read snapshot contains Task-cumulative usage, not enough information
    // to reconstruct an active turn. Keep that counter in `total_tokens` and
    // let live Usage events populate the per-turn fields without relabeling
    // Task history as current-turn work.
    if !state.turn_active {
        state.turn_input_tokens = 0;
        state.turn_output_tokens = 0;
    }
}

fn reduce_event_page(state: &mut AppState, page: SessionEventPage) -> Vec<UiEffect> {
    if page.schema_version != fabric::SESSION_READ_MODEL_SCHEMA_VERSION {
        return vec![UiEffect::AnnounceError(format!(
            "unsupported Session event-page schema {}",
            page.schema_version
        ))];
    }
    if state.session_id.as_deref() != Some(page.session_id.0.as_str()) {
        return Vec::new();
    }
    if page.next.sequence <= state.cursor.sequence {
        return Vec::new();
    }
    if page.after != state.cursor {
        return vec![
            UiEffect::AnnounceError(
                "Session event page is out of order; reloading snapshot".into(),
            ),
            UiEffect::ReloadSnapshot(page.session_id),
        ];
    }

    let mut expected = page.after.sequence;
    for event in &page.events {
        let fabric::protocol::client::ClientEvent::Item(item) = event else {
            return vec![
                UiEffect::AnnounceError(
                    "Session event page contains a non-item projection event".into(),
                ),
                UiEffect::ReloadSnapshot(page.session_id),
            ];
        };
        expected = expected.saturating_add(1);
        if item.cursor.sequence != expected {
            return vec![
                UiEffect::AnnounceError(
                    "Session event page contains a sequence gap; reloading snapshot".into(),
                ),
                UiEffect::ReloadSnapshot(page.session_id),
            ];
        }
    }
    if page.next.sequence != expected
        || page.events.last().is_some_and(|event| match event {
            fabric::protocol::client::ClientEvent::Item(item) => item.cursor != page.next,
            _ => true,
        })
    {
        return vec![
            UiEffect::AnnounceError(
                "Session event page next cursor does not match its ordered tail".into(),
            ),
            UiEffect::ReloadSnapshot(page.session_id),
        ];
    }

    for event in page.events {
        let fabric::protocol::client::ClientEvent::Item(item) = event else {
            unreachable!("event page was validated before mutation")
        };
        apply_item_event(state, item);
    }
    vec![UiEffect::Render, UiEffect::ReloadSnapshot(page.session_id)]
}

fn apply_item_event(state: &mut AppState, event: ItemEvent) {
    state.cursor = event.cursor;
    let id = event.item_id;
    match event.phase {
        ItemPhase::Started => {
            state
                .items
                .entry(id.clone())
                .or_insert_with(|| UiItem::streaming(id));
        }
        ItemPhase::Streaming => {
            let item = state
                .items
                .entry(id.clone())
                .or_insert_with(|| UiItem::streaming(id));
            if item.status != UiItemStatus::Completed {
                item.status = UiItemStatus::Streaming;
                item.content
                    .push_str(event.delta.as_deref().unwrap_or_default());
            }
        }
        ItemPhase::Completed => {
            if let Some(item) = event.item {
                state.items.remove(&id);
                upsert_completed(state, item);
            }
        }
        ItemPhase::Failed => {
            let item = state
                .items
                .entry(id.clone())
                .or_insert_with(|| UiItem::streaming(id));
            item.status = UiItemStatus::Failed;
            item.content = event.error.unwrap_or_else(|| "item failed".into());
        }
    }
}

/// Project the terminal status carried by the canonical client event. This is
/// intentionally transport-neutral and is shared by TUI acceptance fixtures
/// with the ACP projection.
pub fn reduce_terminal(
    state: &mut AppState,
    event: &fabric::protocol::client::ClientEvent,
) -> bool {
    let status = match event {
        fabric::protocol::client::ClientEvent::TurnCompleted { status, stop, .. } => status
            .as_ref()
            .copied()
            .unwrap_or_else(|| fabric::TurnTerminalStatus::from(stop.clone())),
        fabric::protocol::client::ClientEvent::TurnStopped { reason, .. } => {
            fabric::TurnTerminalStatus::from(reason.clone())
        }
        fabric::protocol::client::ClientEvent::Failed { .. } => fabric::TurnTerminalStatus::Failed,
        _ => return false,
    };
    state.last_terminal_status = Some(status);
    state.streaming = false;
    state.turn_active = false;
    true
}

fn advance(state: &mut AppState, cursor: &EventCursor) -> bool {
    if cursor.sequence <= state.cursor.sequence {
        return false;
    }
    state.cursor = cursor.clone();
    true
}

fn upsert_completed(state: &mut AppState, record: ItemRecord) {
    if let ItemPayload::EvaluationReceiptRef { receipt } = &record.payload {
        state.latest_evaluation = Some(receipt.clone());
    }
    let id = record.id.0.to_string();
    let (kind, content, collapsed) = item_content(&record.payload);
    let candidate = UiItem {
        id: id.clone(),
        sequence: record.sequence,
        kind,
        content,
        status: UiItemStatus::Completed,
        collapsed,
    };
    match state.items.get(&id) {
        Some(existing)
            if existing.status == UiItemStatus::Completed
                && existing.sequence >= record.sequence => {}
        _ => {
            state.items.insert(id, candidate);
        }
    }
}

fn item_content(payload: &ItemPayload) -> (String, String, bool) {
    match payload {
        ItemPayload::UserMessage { content } => ("user".into(), content.clone(), false),
        ItemPayload::AssistantMessage { content } => ("assistant".into(), content.clone(), false),
        ItemPayload::ToolCall { name, input, .. } => {
            ("tool_call".into(), format!("{name} {input}"), true)
        }
        ItemPayload::ToolResult {
            content, is_error, ..
        } => (
            if *is_error {
                "tool_error"
            } else {
                "tool_result"
            }
            .into(),
            content.clone(),
            true,
        ),
        ItemPayload::CapabilityReceipt { receipt } => (
            "capability_receipt".into(),
            format!("{}: {:?}", receipt.capability, receipt.status),
            true,
        ),
        ItemPayload::RobotEpisodeReceipt { receipt } => (
            "robot_episode_receipt".into(),
            format!(
                "{}: {} ({})",
                receipt.report().episode_id,
                receipt.report().settlement.as_str(),
                receipt.report_sha256()
            ),
            true,
        ),
        ItemPayload::EvaluationReceiptRef { receipt } => (
            "evaluation_receipt".into(),
            format_evaluation_receipt_ref(receipt),
            true,
        ),
        ItemPayload::ModelContextProjection { receipt } => (
            "model_context_projection".into(),
            format!(
                "{} fragments, {} message bytes, {} tool-schema bytes",
                receipt.fragments.len(),
                receipt.message_bytes,
                receipt.tool_schema_bytes
            ),
            true,
        ),
        ItemPayload::InferenceReceipt { receipt } => (
            "inference_receipt".into(),
            format!("{}: {:?}", receipt.inference_id, receipt.status),
            true,
        ),
        ItemPayload::TaskProjection { .. } => (
            "task_projection".into(),
            "Host Task projection updated".into(),
            true,
        ),
        ItemPayload::TurnRecovery { classification } => (
            "turn_recovery".into(),
            format!("Turn recovery settled: {classification:?}"),
            true,
        ),
        ItemPayload::ContextProjection { space, .. } => ("context".into(), space.clone(), true),
        ItemPayload::SystemNotice { content } => ("system".into(), content.clone(), false),
    }
}

/// Format the public, bounded receipt summary without querying full evidence.
pub fn format_evaluation_receipt_ref(receipt: &EvaluationReceiptRef) -> String {
    let decision = match receipt.decision {
        EvaluationDecision::ObservedPass => "observed_pass",
        EvaluationDecision::ObservedFail => "observed_fail",
        EvaluationDecision::Accepted => "accepted",
        EvaluationDecision::Rejected => "rejected",
        EvaluationDecision::Indeterminate => "indeterminate",
    };
    let score = receipt
        .weighted_total_millis
        .map(|value| format!("{:.1}", f64::from(value) / 1_000.0))
        .unwrap_or_else(|| "unknown".into());
    let coverage = f64::from(receipt.evidence_coverage_millis) / 10.0;
    let confidence = f64::from(receipt.confidence_millis) / 10.0;
    let failed_gates = if receipt.failed_gates.is_empty() {
        "none".into()
    } else {
        receipt.failed_gates.join(",")
    };

    format!(
        "[evaluation] decision={decision} score={score} coverage={coverage:.1}% confidence={confidence:.1}% failed_gates={failed_gates}"
    )
}

#[derive(Debug, Serialize)]
pub struct ReducerSnapshot<'a> {
    pub cursor: u64,
    pub provider: Option<&'a str>,
    pub model: &'a str,
    pub items: Vec<&'a UiItem>,
    pub approvals: Vec<String>,
    pub agents: Vec<String>,
    pub error: Option<&'a str>,
}

pub fn snapshot_view(state: &AppState) -> ReducerSnapshot<'_> {
    ReducerSnapshot {
        cursor: state.cursor.sequence,
        provider: state.provider_name.as_deref(),
        model: &state.model_name,
        items: state.items.values().collect(),
        approvals: state
            .approvals
            .values()
            .map(|value| format!("{}:{:?}", value.id, value.status))
            .collect(),
        agents: state
            .agents
            .values()
            .map(|value| format!("{}:{:?}", value.handle.agent_id.0, value.status))
            .collect(),
        error: state.last_error.as_deref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_projection_keeps_task_usage_separate_from_active_turn_usage() {
        let mut state = AppState::default();
        state.turn_input_tokens = 99;
        state.turn_output_tokens = 9;
        state.tasks.push(fabric::TaskSnapshot {
            task_id: "task-1".into(),
            session_id: fabric::SessionId("session-1".into()),
            goal: None,
            phase: fabric::TaskPhase::Active,
            plan_revision: None,
            steps: Vec::new(),
            active_turn_id: None,
            active_runtime_children: Vec::new(),
            active_commands: Vec::new(),
            pending_approvals: Vec::new(),
            budget: None,
            checkpoint_head: None,
            checkpoint_review: None,
            settlement: None,
            review_findings: Vec::new(),
            runtime_facts: Some(fabric::TaskRuntimeFacts {
                effective_provider: Some("deepseek".into()),
                effective_model: Some("deepseek-v4-flash".into()),
                context_capacity_tokens: Some(1_000_000),
                active_context_occupancy_tokens: Some(8_000),
                cumulative_usage: fabric::InferenceUsage::unsupported(Some(10_000), Some(500)),
                inference_rounds: 2,
                provider_retries: Some(0),
                tool_calls: 1,
                terminal_tool_results: 1,
            }),
        });

        project_runtime_accounting(&mut state);

        assert_eq!(state.context.used, 8_000);
        assert_eq!(state.context.max, 1_000_000);
        assert_eq!(state.total_tokens, 10_500);
        assert_eq!(state.turn_input_tokens, 0);
        assert_eq!(state.turn_output_tokens, 0);

        state.turn_active = true;
        state.context.used = 12_000;
        state.total_tokens = 12_000;
        state.turn_input_tokens = 1_000;
        state.turn_output_tokens = 100;
        project_runtime_accounting(&mut state);

        assert_eq!(state.context.used, 12_000);
        assert_eq!(state.total_tokens, 12_000);
        assert_eq!(state.turn_input_tokens, 1_000);
        assert_eq!(state.turn_output_tokens, 100);
    }
}
