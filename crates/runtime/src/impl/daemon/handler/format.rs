use base::events::ui_event::ClientEvent;
use crate::core::event_sink::Event;

/// Convert a `ClientEvent` to a JSON-RPC 2.0 notification string.
pub fn event_to_json(event: &ClientEvent) -> serde_json::Result<String> {
    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": event,
    });
    serde_json::to_string(&notification)
}

/// Convert an internal `Event` to a client-facing `ClientEvent`, if applicable.
/// Returns `None` for internal-only events (compaction, memory, approval, etc.).
pub fn event_to_client_event(event: &Event) -> Option<ClientEvent> {
    match event {
        Event::TurnStarted { iteration } => Some(ClientEvent::TurnStarted { iteration: *iteration }),
        Event::TextDelta { delta } => Some(ClientEvent::TextDelta { text: delta.clone() }),
        Event::ToolCallStart { name, call_id } => Some(ClientEvent::ToolCallStart {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: serde_json::Value::Null,
        }),
        Event::ToolCallComplete { call_id, name, args } => Some(ClientEvent::ToolCallComplete {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: args.clone(),
        }),
        Event::ToolResult { name, call_id, result } => Some(ClientEvent::ToolCallResult {
            call_id: call_id.clone(),
            tool: name.clone(),
            output: result.content.clone(),
            is_error: result.is_error,
            elapsed_ms: result.execution_time_ms,
        }),
        Event::Usage { tokens_in, tokens_out, .. } => Some(ClientEvent::Usage {
            tokens_in: *tokens_in as u64,
            tokens_out: *tokens_out as u64,
        }),
        Event::TurnDone { .. } => Some(ClientEvent::TurnDone),
        Event::Error { message } => Some(ClientEvent::Error { message: message.clone() }),
        Event::AwarenessChanged { level, context } => Some(ClientEvent::AwarenessChanged {
            level: level.clone(),
            context: context.clone(),
        }),
        Event::ModeChanged { mode } => Some(ClientEvent::ModeChanged { new: mode.clone() }),
        Event::SubAgentStatusChanged { agent_id, status, task } => Some(ClientEvent::SubAgentStatus {
            agent_id: agent_id.clone(),
            task: task.clone(),
            status: status.clone(),
        }),
        Event::PlanUpdate { version, plan, critique, ready_for_approval } => Some(ClientEvent::PlanUpdate {
            version: *version,
            plan: plan.clone(),
            critique: critique.clone(),
            ready_for_approval: *ready_for_approval,
        }),
        Event::Interrupted { .. } => Some(ClientEvent::Interrupted),
        Event::ContextUpdate { used_tokens, max_tokens } => Some(ClientEvent::ContextUpdate {
            used_tokens: *used_tokens as u64,
            max_tokens: *max_tokens as u64,
        }),
        Event::ModelSwitch { model_name } => Some(ClientEvent::ModelSwitch {
            model: model_name.clone(),
        }),
        Event::GoalSet { goal, sub_goals } => Some(ClientEvent::GoalSet {
            goal: goal.clone(),
            sub_goals: sub_goals.clone(),
        }),
        Event::Reflection { summary, .. } => Some(ClientEvent::Reflection {
            summary: summary.clone(),
        }),
        Event::BudgetExceeded { max, .. } => Some(ClientEvent::BudgetExceeded {
            limit: *max as u64,
        }),
        Event::CircuitBreakerTripped { reason } => Some(ClientEvent::CircuitBreakerTripped {
            reason: reason.clone(),
        }),
        Event::CompactionTriggered { .. } => Some(ClientEvent::CompactionTriggered),

        // Internal-only events — not for client consumption.
        Event::Text { .. }
        | Event::Reasoning { .. }
        | Event::ToolDispatch { .. }
        | Event::ApprovalRequest { .. }
        | Event::AskRequest { .. }
        | Event::CompactionStarted
        | Event::CompactionDone { .. }
        | Event::MemoryUpdated { .. }
        | Event::PlanModeChanged { .. }
        | Event::CacheDiagnostics { .. } => None,
    }
}

/// Expand leading `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
}
