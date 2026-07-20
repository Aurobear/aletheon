//! Pure protocol-to-view-state reducer.

use fabric::protocol::client::{
    AgentEvent, ApprovalEvent, EventCursor, ItemEvent, ItemPhase, UiSnapshot,
};
use fabric::{ItemPayload, ItemRecord};
use serde::Serialize;

use super::state::{AppState, UiItem, UiItemStatus};

#[derive(Debug, Clone)]
pub enum UiAction {
    Snapshot(UiSnapshot),
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
        UiAction::Item(event) => {
            if event.cursor.sequence <= state.cursor.sequence {
                return Vec::new();
            }
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
        ItemPayload::ContextProjection { space, .. } => ("context".into(), space.clone(), true),
        ItemPayload::SystemNotice { content } => ("system".into(), content.clone(), false),
    }
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
