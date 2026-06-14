use tracing::{info, warn};

use aletheon_brain_core::r#impl::learning::{OutcomeRecord, OutcomeContext};
use aletheon_abi::tool::ToolResult;

use super::cognitive_loop::Engine;

impl Engine {
    /// Record a tool call outcome for the learning pipeline.
    pub(super) async fn record_tool_outcome(
        &mut self,
        session_id: &str,
        turn_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
        result: &ToolResult,
        iteration: usize,
    ) {
        let record = OutcomeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_name: tool_name.to_string(),
            args: tool_input.clone(),
            result_summary: if result.content.len() > 500 {
                format!("{}...", &result.content[..500])
            } else {
                result.content.clone()
            },
            is_error: result.is_error,
            user_feedback: None,
            timestamp: chrono::Utc::now(),
            context: OutcomeContext {
                preceding_errors: 0,
                iteration_count: iteration,
                system_state: None,
            },
        };

        // Record to learning SQLite database
        if let Some(ref recorder) = self.outcome_recorder {
            if let Err(e) = recorder.record(&record) {
                warn!(error = %e, "Failed to record tool outcome");
            }
        }

        // Also store in recall memory for cross-session persistence
        {
            let rm = self.recall_memory.lock().await;
            let metadata = serde_json::json!({
                "type": "learning_outcome",
                "tool": tool_name,
                "is_error": result.is_error,
                "iteration": iteration,
            });
            let label = if result.is_error { "ERROR" } else { "OK" };
            let content = format!("[{}] {}: {}", label, tool_name, record.result_summary);
            if let Err(e) = rm.store(session_id, "learning_outcome", &content, Some(&metadata.to_string())) {
                warn!(error = %e, "Failed to store learning outcome in recall memory");
            }
        }

        // Extract patterns from recent outcomes and update rule store
        if let (Some(ref recorder), Some(ref extractor)) = (&self.outcome_recorder, &self.pattern_extractor) {
            if let Ok(recent_outcomes) = recorder.get_recent(100) {
                let new_rules = extractor.extract(&recent_outcomes);
                for rule in new_rules {
                    info!(rule_id = %rule.id, rule_type = %rule.rule_type, tool = %rule.tool_pattern, "Adding learned rule");
                    self.rule_store.add(rule);
                }
            }
        }
    }
}
