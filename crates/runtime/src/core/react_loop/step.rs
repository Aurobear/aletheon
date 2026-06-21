use super::{is_context_overflow, ReActLoop, TurnMetrics};
use crate::core::event_sink::EventSink;

use base::message::{ContentBlock, Message, Role};
use base::ToolDefinition;
use cognit::r#impl::llm::provider::{LlmProvider, StopReason};
use std::future::Future;
use tracing::{debug, warn};

impl ReActLoop {
    /// Run the interleaved ReAct loop: call the LLM with tools, execute any
    /// requested tools via `execute_tool`, feed results back, and repeat until
    /// the LLM stops requesting tools or `max_iterations` is reached.
    pub async fn run<L, F, Fut>(
        &mut self,
        user_input: &str,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider + ?Sized,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        let start = std::time::Instant::now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;

        self.messages.push(Message::user(user_input));

        while self.should_continue() {
            self.advance();
            self.emit_loop_start(&format!("iter_{}", self.iteration));
            let response = match llm.complete(&self.messages, tool_defs).await {
                Ok(r) => r,
                Err(e) if is_context_overflow(&e) => {
                    // A3: reactive compaction on context overflow
                    warn!("Context overflow detected, forcing compaction: {e}");
                    self.compressor
                        .maybe_compact(&mut self.messages, llm)
                        .await?;
                    llm.complete(&self.messages, tool_defs).await?
                }
                Err(e) => return Err(e),
            };

            let mut text_parts = Vec::new();
            let mut thinking_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::Thinking { text, .. } => {
                        thinking_parts.push(text.clone());
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            if tool_calls.is_empty() || matches!(response.stop_reason, StopReason::EndTurn) {
                let final_text = text_parts.join("\n");
                // Emit awareness: uncertainty from response + final response signal
                self.emit_thinking_complete("thinking", &final_text);
                self.emit_final_response("final_response");
                self.messages.push(Message::assistant(&final_text));
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
            }

            // Record the assistant turn (text + tool_use blocks) verbatim.
            self.messages.push(Message {
                role: Role::Assistant,
                content: response.content.clone(),
            });

            // Execute each requested tool and feed results back.
            for (id, name, input) in &tool_calls {
                debug!(tool = name.as_str(), "ReActLoop executing tool");
                let (content, is_error) = execute_tool(id, name, input).await;
                tool_calls_made += 1;
                if is_error {
                    tool_errors += 1;
                    self.consecutive_errors += 1;
                    warn!(tool = name.as_str(), "tool returned error");
                } else {
                    self.consecutive_errors = 0;
                }
                // Emit awareness signal for tool completion
                self.emit_tool_call_end(name);
                self.messages
                    .push(Message::tool_result(id, &content, is_error));
            }

            // A2: proactive compaction after pushing tool results
            if self.config.compaction_enabled {
                let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
            }
        }

        warn!(
            max = self.config.max_iterations,
            "ReActLoop hit max_iterations"
        );
        let final_text = self
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations));
        let metrics = TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((final_text, metrics))
    }
}
