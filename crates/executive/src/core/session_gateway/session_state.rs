//! Session state management for SessionGateway.
//!
//! Contains `SessionStateRef` (the runtime state snapshot struct),
//! state synchronization methods, and the `session.state` query handler.

use serde_json::{json, Value};

use cognit::harness::linear::circuit_breaker::CircuitBreakerStatus;
use cognit::harness::linear::goal_tracker::GoalTracker;

use super::gateway::SessionGateway;

/// Lightweight reference to SessionState internals for snapshot queries.
/// Avoids circular dependency between session_gateway and handler modules.
pub struct SessionStateRef {
    pub iteration: usize,
    pub plan_mode: bool,
    pub consecutive_errors: usize,
    pub circuit_breaker_status: CircuitBreakerStatus,
    pub tool_budget_remaining: usize,
    pub tool_budget_max: usize,
    pub recent_tools: Vec<String>,
    pub storm_breaker_failure_count: usize,
    pub goal_tracker: GoalTracker,
}

impl SessionGateway {
    /// Update the snapshot state ref with current ReActLoop state.
    /// Called by the handler after each turn to keep snapshot data fresh.
    pub async fn update_state(&self, new_state: SessionStateRef) {
        let mut guard = self.state.lock().await;
        *guard = new_state;
    }

    /// Update the session state after a turn completes.
    /// Called by the handler after each ReActLoop turn to keep snapshot data fresh.
    pub async fn update_turn_state(
        &self,
        iteration: usize,
        consecutive_errors: usize,
        tool_calls_made: usize,
        recent_tools: Vec<String>,
        storm_breaker_failure_count: usize,
        goal_description: Option<String>,
    ) {
        let mut guard = self.state.lock().await;
        guard.iteration = iteration;
        guard.consecutive_errors = consecutive_errors;
        // Decrement tool budget by calls made this turn
        if tool_calls_made <= guard.tool_budget_remaining {
            guard.tool_budget_remaining -= tool_calls_made;
        } else {
            guard.tool_budget_remaining = 0;
        }
        if !recent_tools.is_empty() {
            guard.recent_tools = recent_tools;
        }
        guard.storm_breaker_failure_count = storm_breaker_failure_count;
        if let Some(goal) = goal_description {
            let current = guard.goal_tracker.current_goal_description();
            if current.as_deref() != Some(&goal) {
                guard.goal_tracker.set_goal(goal);
            }
        }
    }

    // ── Phase C: session.state handler ─────────────────────────────────────

    pub(crate) async fn handle_state(&self, id: &Value) -> Value {
        let state = self.state.lock().await;
        let messages = self.session_manager.lock().await;

        let mut md = String::from("# ReActLoop State\n\n");

        md.push_str("## Loop State\n");
        md.push_str(&format!("- Iteration: {}\n", state.iteration));
        md.push_str(&format!(
            "- Max iterations: {}\n",
            self.runtime_config.max_iterations
        ));
        md.push_str(&format!(
            "- Plan mode: {}\n",
            if state.plan_mode { "yes" } else { "no" }
        ));
        md.push_str(&format!(
            "- Consecutive errors: {}\n",
            state.consecutive_errors
        ));

        md.push_str("\n## Tool Budget\n");
        md.push_str(&format!(
            "- Used: {} / {}\n",
            state.tool_budget_max - state.tool_budget_remaining,
            state.tool_budget_max
        ));
        if !state.recent_tools.is_empty() {
            md.push_str("- Recent tools:\n");
            for t in state.recent_tools.iter().rev().take(10) {
                md.push_str(&format!("  - {}\n", t));
            }
        }

        md.push_str("\n## Circuit Breaker\n");
        md.push_str(&format!("- Status: {:?}\n", state.circuit_breaker_status));

        md.push_str("\n## Goal Tracker\n");
        md.push_str(&state.goal_tracker.get_context());
        md.push('\n');

        md.push_str("## Session\n");
        md.push_str(&format!("- Messages: {}\n", messages.message_count()));
        md.push_str(&format!(
            "- Estimated tokens: {}\n\n",
            messages.estimate_tokens()
        ));

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": md }
        })
    }
}
