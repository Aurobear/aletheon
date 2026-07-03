use crate::core::event_sink::Event;
use serde_json::json;

/// Convert an `Event` to a JSONL string for the notify channel.
/// Returns `None` for events that don't have a client-facing representation.
pub fn event_to_json(event: &Event) -> Option<String> {
    let params = match event {
        Event::TurnStarted { iteration } => json!({"type": "turn_start", "iteration": iteration}),
        Event::TextDelta { delta } => json!({"type": "text_delta", "text": delta}),
        Event::ToolCallStart { name, call_id } => {
            json!({"type": "tool_call_start", "call_id": call_id, "tool": name})
        }
        Event::ToolResult {
            name,
            call_id,
            result,
        } => json!({
            "type": "tool_call_result",
            "call_id": call_id,
            "tool": name,
            "output": result.content,
            "is_error": result.is_error,
            "elapsed_ms": result.execution_time_ms,
        }),
        Event::ToolDispatch { name, args } => {
            json!({"type": "tool_dispatch", "tool": name, "args": args})
        }
        Event::Usage {
            tokens_in,
            tokens_out,
            ..
        } => json!({"type": "usage", "tokens_in": tokens_in, "tokens_out": tokens_out}),
        Event::TurnDone { result } => json!({"type": "turn_done", "success": result.is_ok()}),
        Event::Error { message } => json!({"type": "error", "message": message}),
        Event::AwarenessChanged { level, context } => json!({
            "type": "awareness_changed",
            "level": level,
            "context": context,
        }),
        Event::ModeChanged { mode } => json!({
            "type": "mode_changed",
            "mode": mode,
        }),
        Event::SubAgentStatusChanged {
            agent_id,
            status,
            task,
        } => json!({
            "type": "sub_agent_status",
            "agent_id": agent_id,
            "status": status,
            "task": task,
        }),
        Event::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        } => json!({
            "type": "plan_update",
            "version": version,
            "plan": plan,
            "critique": critique,
            "ready_for_approval": ready_for_approval,
        }),
        Event::Interrupted { reason } => json!({
            "type": "interrupted",
            "reason": reason,
        }),
        Event::ContextUpdate {
            used_tokens,
            max_tokens,
        } => json!({
            "type": "context_update",
            "used_tokens": used_tokens,
            "max_tokens": max_tokens,
        }),
        Event::ModelSwitch { model_name } => json!({
            "type": "model_switch",
            "model_name": model_name,
        }),
        Event::GoalSet { goal, sub_goals } => json!({
            "type": "goal_set",
            "goal": goal,
            "sub_goals": sub_goals,
        }),
        Event::Reflection {
            summary,
            recommendation,
        } => json!({
            "type": "reflection",
            "summary": summary,
            "recommendation": recommendation,
        }),
        Event::BudgetExceeded { used, max } => json!({
            "type": "budget_exceeded",
            "used": used,
            "max": max,
        }),
        Event::CircuitBreakerTripped { reason } => json!({
            "type": "circuit_breaker_tripped",
            "reason": reason,
        }),
        Event::CompactionTriggered {
            used_tokens,
            threshold,
            reason,
        } => json!({
            "type": "compaction_triggered",
            "used_tokens": used_tokens,
            "threshold": threshold,
            "reason": reason,
        }),
        _ => return None,
    };
    Some(json!({"jsonrpc": "2.0", "method": "event", "params": params}).to_string())
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
