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
    state.last_error = None;
    vec![
        UiEffect::Render,
        UiEffect::SubscribeAfter(state.cursor.clone()),
    ]
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
