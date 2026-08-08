//! Pure protocol-to-view-state reducer.

use fabric::protocol::client::{
    ActivityKind, ActivitySnapshot, ActivityState, AgentEvent, ApprovalEvent, EventCursor,
    ItemEvent, ItemPhase, SessionEventPage, SessionReadSnapshot, UiSnapshot,
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
    LiveActivity(LiveActivityEvent),
    /// Ephemeral assistant text that has not yet been committed as a durable
    /// item. It is rendered on the canonical conversation surface before the
    /// durable projection replaces it, so live output is visible (U1-AUDIT-001).
    LiveAssistantText {
        text: String,
        sequence: u64,
    },
}

#[derive(Debug, Clone)]
pub enum LiveActivityEvent {
    InferenceStarted {
        iteration: usize,
        observed_at: u64,
    },
    InferenceFinished {
        iteration: usize,
        observed_at: u64,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        args: serde_json::Value,
        observed_at: u64,
    },
    ToolArguments {
        call_id: String,
        args: serde_json::Value,
        observed_at: u64,
    },
    ToolProgress {
        call_id: String,
        payload: serde_json::Value,
        observed_at: u64,
    },
    ToolFinished {
        call_id: String,
        is_error: bool,
        elapsed_ms: u64,
        observed_at: u64,
    },
    ProgressSummary {
        summary: String,
        observed_at: u64,
    },
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
            clear_ephemeral_overlays(state);
            if cursor.sequence > state.cursor.sequence {
                state.cursor = cursor;
            }
            let mut effects = Vec::new();
            if let Some(session_id) = state.session_id.clone() {
                effects.push(UiEffect::ReloadSnapshot(fabric::SessionId(session_id)));
            }
            effects.push(UiEffect::SubscribeAfter(state.cursor.clone()));
            effects
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
        UiAction::LiveActivity(event) => {
            reduce_live_activity(state, event);
            vec![UiEffect::Render]
        }
        UiAction::LiveAssistantText { text, sequence } => {
            reduce_live_assistant_text(state, text, sequence);
            vec![UiEffect::Render]
        }
    }
}

/// Establish one stable identity for a live turn before applying any
/// ephemeral text or activity. Versioned events provide the authoritative
/// identity; the V0 compatibility stream receives a reducer-local identity
/// that is never projected as durable truth.
pub fn begin_live_turn(state: &mut AppState, turn_id: Option<fabric::TurnId>) {
    let current = state.active_turn_id.or(state.live_turn_id);
    let next = turn_id.unwrap_or_else(|| {
        if state.turn_active {
            current.unwrap_or_default()
        } else {
            fabric::TurnId::new()
        }
    });
    if current != Some(next) {
        clear_ephemeral_overlays(state);
    }
    state.active_turn_id = turn_id;
    state.live_turn_id = turn_id.is_none().then_some(next);
    state.turn_active = true;
}

/// Upsert an ephemeral assistant item so live streamed text is visible on the
/// canonical conversation surface before the durable projection replaces it.
/// The durable item (same stable id once committed) supersedes this overlay;
/// until then it is the only visible representation.
fn reduce_live_assistant_text(state: &mut AppState, text: String, sequence: u64) {
    ensure_live_turn_id(state);
    let live_assistant_id = live_assistant_id(state);
    let live = UiItem {
        id: live_assistant_id.clone(),
        sequence,
        kind: "assistant".into(),
        content: text,
        status: UiItemStatus::Streaming,
        collapsed: false,
    };
    // A live overlay must never displace a durable item with the same sequence.
    // If the durable projection already committed this turn's assistant text,
    // drop the ephemeral copy so no duplication can occur.
    let durable_at_or_after = state.items.values().any(|item| {
        item.kind == "assistant" && item.sequence >= sequence && item.id != live_assistant_id
    });
    if durable_at_or_after {
        state.items.remove(&live_assistant_id);
        return;
    }
    state.items.insert(live_assistant_id, live);
}

/// Remove the ephemeral live-assistant overlay once a durable assistant item
/// commits, so the same text is atomically replaced rather than duplicated.
fn clear_live_assistant_overlay(state: &mut AppState, record: &ItemRecord) {
    let canonical_id = format!(
        "live:{}:{}:assistant",
        record.session_id.0, record.turn_id.0
    );
    state.items.retain(|id, _| {
        id != &canonical_id
            && !(id.starts_with("live:") && id.ends_with(":assistant"))
            && !(id.starts_with("local:") && id.contains(":assistant:"))
    });
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

    let session_changed = state.session_id.as_deref() != Some(snapshot.session.id.0.as_str());
    if session_changed {
        clear_ephemeral_overlays(state);
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
    reconcile_live_activities(state, snapshot.activities);
    project_runtime_accounting(state);
    state.last_error = None;
    vec![
        UiEffect::Render,
        UiEffect::SubscribeAfter(state.cursor.clone()),
    ]
}

fn live_scope(state: &AppState) -> String {
    format!(
        "{}:{}",
        state.session_id.as_deref().unwrap_or("unbound-session"),
        active_turn_id(state).0
    )
}

fn live_assistant_id(state: &AppState) -> String {
    format!("live:{}:assistant", live_scope(state))
}

fn live_activity_id(state: &AppState, call_id: &str) -> String {
    format!("live:{}:tool:{call_id}", live_scope(state))
}

fn reduce_live_activity(state: &mut AppState, event: LiveActivityEvent) {
    ensure_live_turn_id(state);
    match event {
        LiveActivityEvent::InferenceStarted {
            iteration,
            observed_at,
        } => {
            if iteration == 0 {
                let scope_prefix = format!("live:{}:", live_scope(state));
                state
                    .activities
                    .retain(|activity| !activity.activity_id.starts_with(&scope_prefix));
                state
                    .live_activity_ids
                    .retain(|activity_id| !activity_id.starts_with(&scope_prefix));
            }
            let activity_id = format!("live:{}:inference:{iteration}", live_scope(state));
            state.live_activity_ids.insert(activity_id.clone());
            state.activities.push(ActivitySnapshot {
                activity_id,
                task_id: active_task_id(state),
                turn_id: active_turn_id(state),
                parent_activity_id: None,
                kind: ActivityKind::Runtime,
                label: format!("Model inference round {}", iteration + 1),
                state: ActivityState::Running,
                started_at: observed_at,
                updated_at: observed_at,
                progress: Some(serde_json::json!("reasoning and selecting next action")),
                artifact_refs: Vec::new(),
                receipt_ref: None,
            });
        }
        LiveActivityEvent::InferenceFinished {
            iteration,
            observed_at,
        } => {
            let activity_id = format!("live:{}:inference:{iteration}", live_scope(state));
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                activity.state = ActivityState::Completed;
                activity.updated_at = observed_at;
                activity.progress = Some(serde_json::json!("provider response received"));
            }
        }
        LiveActivityEvent::ToolStarted {
            call_id,
            tool,
            args,
            observed_at,
        } => {
            let activity_id = live_activity_id(state, &call_id);
            state.live_activity_ids.insert(activity_id.clone());
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                activity.label = tool;
                activity.state = ActivityState::Running;
                activity.updated_at = observed_at;
                activity.progress = Some(serde_json::json!({
                    "status": "running",
                    "args": args,
                }));
            } else {
                state.activities.push(ActivitySnapshot {
                    activity_id,
                    task_id: active_task_id(state),
                    turn_id: active_turn_id(state),
                    parent_activity_id: None,
                    kind: ActivityKind::Tool,
                    label: tool,
                    state: ActivityState::Running,
                    started_at: observed_at,
                    updated_at: observed_at,
                    progress: Some(serde_json::json!({
                        "status": "running",
                        "args": args,
                    })),
                    artifact_refs: Vec::new(),
                    receipt_ref: None,
                });
            }
        }
        LiveActivityEvent::ToolArguments {
            call_id,
            args,
            observed_at,
        } => {
            let activity_id = live_activity_id(state, &call_id);
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                let mut progress = activity
                    .progress
                    .take()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                progress.insert("args".into(), args);
                progress.insert("status".into(), serde_json::json!("running"));
                activity.progress = Some(serde_json::Value::Object(progress));
                activity.updated_at = observed_at;
            }
        }
        LiveActivityEvent::ToolProgress {
            call_id,
            payload,
            observed_at,
        } => {
            let activity_id = live_activity_id(state, &call_id);
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                activity.state = ActivityState::Running;
                activity.updated_at = observed_at;
                activity.progress = Some(payload);
            }
        }
        LiveActivityEvent::ToolFinished {
            call_id,
            is_error,
            elapsed_ms,
            observed_at,
        } => {
            let activity_id = live_activity_id(state, &call_id);
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                activity.state = if is_error {
                    ActivityState::Failed
                } else {
                    ActivityState::Completed
                };
                activity.updated_at = observed_at;
                let mut progress = activity
                    .progress
                    .take()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                progress.insert(
                    "status".into(),
                    serde_json::json!(if is_error { "failed" } else { "completed" }),
                );
                progress.insert("elapsed_ms".into(), serde_json::json!(elapsed_ms));
                activity.progress = Some(serde_json::Value::Object(progress));
            }
        }
        LiveActivityEvent::ProgressSummary {
            summary,
            observed_at,
        } => {
            let activity_id = format!("live:{}:progress", live_scope(state));
            let summary = bounded_summary(&summary, 160);
            state.live_activity_ids.insert(activity_id.clone());
            if let Some(activity) = state
                .activities
                .iter_mut()
                .find(|activity| activity.activity_id == activity_id)
            {
                activity.label = summary;
                activity.state = ActivityState::Running;
                activity.updated_at = observed_at;
                activity.progress = Some(serde_json::json!("strategy update"));
            } else {
                state.activities.push(ActivitySnapshot {
                    activity_id,
                    task_id: active_task_id(state),
                    turn_id: active_turn_id(state),
                    parent_activity_id: None,
                    kind: ActivityKind::Runtime,
                    label: summary,
                    state: ActivityState::Running,
                    started_at: observed_at,
                    updated_at: observed_at,
                    progress: Some(serde_json::json!("strategy update")),
                    artifact_refs: Vec::new(),
                    receipt_ref: None,
                });
            }
        }
    }
}

fn active_task(state: &AppState) -> Option<&fabric::TaskSnapshot> {
    state
        .tasks
        .iter()
        .find(|task| {
            matches!(
                task.phase,
                fabric::TaskPhase::Active | fabric::TaskPhase::Interrupted
            )
        })
        .or_else(|| state.tasks.first())
}

fn active_task_id(state: &AppState) -> String {
    active_task(state)
        .map(|task| task.task_id.clone())
        .unwrap_or_else(|| "live-turn".into())
}

fn active_turn_id(state: &AppState) -> fabric::TurnId {
    state
        .active_turn_id
        .or(state.live_turn_id)
        .or_else(|| active_task(state).and_then(|task| task.active_turn_id))
        .expect("live reducer actions establish a stable turn identity")
}

fn ensure_live_turn_id(state: &mut AppState) {
    if state.active_turn_id.is_none() && state.live_turn_id.is_none() {
        state.live_turn_id = Some(fabric::TurnId::new());
    }
}

fn bounded_summary(summary: &str, max_chars: usize) -> String {
    let normalized = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        format!(
            "{}…",
            normalized
                .chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn reconcile_live_activities(state: &mut AppState, durable: Vec<ActivitySnapshot>) {
    let mut live = state
        .activities
        .drain(..)
        .filter(|activity| state.live_activity_ids.contains(&activity.activity_id))
        .map(|activity| (activity.activity_id.clone(), activity))
        .collect::<std::collections::BTreeMap<_, _>>();

    for activity in &durable {
        if activity.kind != ActivityKind::Tool {
            continue;
        }
        live.retain(|live_id, _| {
            let Some((_, call_id)) = live_id.rsplit_once(":tool:") else {
                return true;
            };
            !activity.activity_id.ends_with(&format!(":{call_id}"))
        });
    }

    state.activities = durable;
    state.activities.extend(live.into_values());
    state.live_activity_ids = state
        .activities
        .iter()
        .filter(|activity| activity.activity_id.starts_with("live:"))
        .map(|activity| activity.activity_id.clone())
        .collect();
}

fn project_runtime_accounting(state: &mut AppState) {
    let facts = active_task(state).and_then(|task| task.runtime_facts.clone());
    let Some(facts) = facts else {
        if !state.turn_active {
            state.context = super::state::ContextDisplay::default();
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
        if state.context.max.is_none() {
            state.context.max = facts
                .context_capacity_tokens
                .and_then(|value| usize::try_from(value).ok());
        }
    } else {
        state.context.used = facts
            .active_context_occupancy_tokens
            .and_then(|value| usize::try_from(value).ok());
        state.context.max = facts
            .context_capacity_tokens
            .and_then(|value| usize::try_from(value).ok());
        state.total_tokens = projected_total;
        // A read snapshot contains Task-cumulative usage, not enough
        // information to reconstruct one active turn.
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
                if item_content(&item.payload).0 == "assistant" {
                    clear_live_assistant_overlay(state, &item);
                }
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

fn clear_ephemeral_overlays(state: &mut AppState) {
    state.items.retain(|id, item| {
        !id.starts_with("live:")
            && !id.starts_with("local:")
            && item.status != UiItemStatus::Streaming
    });
    state
        .activities
        .retain(|activity| !state.live_activity_ids.contains(&activity.activity_id));
    state.live_activity_ids.clear();
    state.live_turn_id = None;
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
    finish_live_turn(state, status);
    true
}

/// Settle reducer-local progress when the compatibility stream reaches its
/// terminal boundary. Durable conversation and task truth still arrive through
/// the Session projection; this only prevents an already-finished turn from
/// remaining visibly `running` while that projection catches up.
pub fn finish_live_turn(state: &mut AppState, status: fabric::TurnTerminalStatus) {
    let activity_state = match status {
        fabric::TurnTerminalStatus::Completed => ActivityState::Completed,
        fabric::TurnTerminalStatus::Interrupted => ActivityState::Cancelled,
        fabric::TurnTerminalStatus::Failed => ActivityState::Failed,
    };
    for activity in &mut state.activities {
        if state.live_activity_ids.contains(&activity.activity_id)
            && activity.state == ActivityState::Running
        {
            activity.state = activity_state;
        }
    }
    state.last_terminal_status = Some(status);
    state.streaming = false;
    state.turn_active = false;
    state.active_turn_id = None;
    state.live_turn_id = None;
}

fn advance(state: &mut AppState, cursor: &EventCursor) -> bool {
    if cursor.sequence <= state.cursor.sequence {
        return false;
    }
    state.cursor = cursor.clone();
    true
}

fn upsert_completed(state: &mut AppState, record: ItemRecord) {
    if let ItemPayload::UserMessage {
        execution_target, ..
    } = &record.payload
    {
        state.reconcile_projected_execution_target(execution_target, record.sequence);
    }
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
        ItemPayload::UserMessage { content, .. } => ("user".into(), content.clone(), false),
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
        ItemPayload::ContextBudgetProjection { projection } => (
            "context_budget_projection".into(),
            format!(
                "{}: window {} tokens, profile input {} tokens, history {}/{} tokens",
                projection.model_spec,
                projection.model_context_tokens.get(),
                projection.profile_input_limit_tokens.get(),
                projection.current_history_tokens.get(),
                projection.admissible_history_tokens.get(),
            ),
            true,
        ),
        ItemPayload::ContextCompactionProjection { projection } => (
            "context_compaction_projection".into(),
            format!(
                "{:?}: {} -> {} tokens at {}",
                projection.mode,
                projection.tokens_before.get(),
                projection.tokens_after.get(),
                projection.trigger_threshold_tokens.get(),
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
        ItemPayload::TurnSettlement { status, content } => {
            ("assistant".into(), format!("{content} ({status:?})"), false)
        }
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
                context_budget: None,
                cumulative_usage: fabric::InferenceUsage::unsupported(Some(10_000), Some(500)),
                inference_rounds: 2,
                provider_retries: Some(0),
                tool_calls: 1,
                terminal_tool_results: 1,
            }),
        });

        project_runtime_accounting(&mut state);

        assert_eq!(state.context.used, Some(8_000));
        assert_eq!(state.context.max, Some(1_000_000));
        assert_eq!(state.total_tokens, 10_500);
        assert_eq!(state.turn_input_tokens, 0);
        assert_eq!(state.turn_output_tokens, 0);

        state.turn_active = true;
        state.context.used = Some(12_000);
        state.total_tokens = 12_000;
        state.turn_input_tokens = 1_000;
        state.turn_output_tokens = 100;
        project_runtime_accounting(&mut state);

        assert_eq!(state.context.used, Some(12_000));
        assert_eq!(state.total_tokens, 12_000);
        assert_eq!(state.turn_input_tokens, 1_000);
        assert_eq!(state.turn_output_tokens, 100);
    }
}
