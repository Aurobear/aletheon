use super::{is_context_overflow, ReActLoop, TurnMetrics};
use crate::core::event_sink::{Event, EventSink};
use super::tool_budget;
use super::circuit_breaker::{CircuitBreakerStatus, ToolCallSignature};

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

            // No tool calls -> turn complete.
            // Note: some models return EndTurn even when tool calls are present.
            // We must check tool_calls first — only exit if there are no tools to run.
            if tool_calls.is_empty() {
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

            // Deferred reflection — injected after all tool results to preserve
            // OpenAI API message format (assistant(tool_use) → tool results only)
            let mut pending_reflection: Option<String> = None;

            // Execute each requested tool and feed results back.
            for (id, name, input) in &tool_calls {
                // Check tool budget before executing
                if !self.tool_budget.can_call() {
                    warn!("Tool budget exhausted, stopping loop");
                    let msg = format!(
                        "Tool budget exhausted after {} calls. Partial result: {}",
                        self.tool_budget.total_calls(),
                        text_parts.join(" ")
                    );
                    let metrics = TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        iterations: self.iteration,
                        completed_normally: false,
                    };
                    return Ok((msg, metrics));
                }

                // Check circuit breaker before executing
                let signature = ToolCallSignature::new(name, input);
                match self.circuit_breaker.check(&signature) {
                    CircuitBreakerStatus::Tripped(reason) => {
                        warn!("Circuit breaker tripped: {}", reason);
                        let msg = format!("Loop detected: {}. Stopping.", reason);
                        let metrics = TurnMetrics {
                            tool_calls_made,
                            tool_errors,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            iterations: self.iteration,
                            completed_normally: false,
                        };
                        return Ok((msg, metrics));
                    }
                    CircuitBreakerStatus::Warning(reason) => {
                        warn!("Circuit breaker warning: {}", reason);
                    }
                    CircuitBreakerStatus::Ok => {}
                }

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
                // Record call in budget tracker (for analytics/history only;
                // the can_call() guard above already enforces the budget limit)
                self.tool_budget.record_call(tool_budget::ToolCallRecord {
                    tool_name: name.clone(),
                    timestamp: std::time::Instant::now(),
                    success: !is_error,
                });
                // Emit awareness signal for tool completion
                self.emit_tool_call_end(name);
                // Record call in reflection engine (defer injection until after
                // all tool results to preserve OpenAI API message format)
                let mut should_reflect = false;
                if self.reflection_engine.record_call() {
                    should_reflect = true;
                }
                // Truncate large tool outputs before storing in conversation
                const MAX_TOOL_RESULT_CHARS: usize = 8000;
                let truncated_content = if content.len() > MAX_TOOL_RESULT_CHARS {
                    let head = &content[..content.char_indices().nth(3500).map(|(i, _)| i).unwrap_or(3500)];
                    let tail_start = content.char_indices().nth(content.chars().count().saturating_sub(3500)).map(|(i, _)| i).unwrap_or(0);
                    let tail = &content[tail_start..];
                    format!("{}\n... ({} chars truncated) ...\n{}", head, content.len() - 7000, tail)
                } else {
                    content.clone()
                };
                self.messages
                    .push(Message::tool_result(id, &truncated_content, is_error));
                // Defer reflection until after all tool results
                if should_reflect {
                    let ctx = crate::core::react_loop::reflection::ReflectionContext {
                        goal: self.goal_tracker.current_goal_description(),
                        recent_actions: self.recent_tools.clone(),
                        current_state: if is_error { "error" } else { "ok" }.to_string(),
                        tool_calls_made,
                        errors: tool_errors,
                        constraints: Vec::new(),
                        test_failures: Vec::new(),
                        unexpected_outputs: Vec::new(),
                    };
                    let result = self.reflection_engine.reflect(&ctx);
                    // Store for injection after all tool results
                    pending_reflection = Some(result.summary);
                }
            }

            // Inject reflection AFTER all tool results to preserve API message format
            if let Some(summary) = pending_reflection.take() {
                self.messages.push(Message::user(format!("[Reflection]\n{}", summary)));
            }

            // Check if reflection recommended stopping
            if self.reflection_engine.should_stop() {
                let final_text = text_parts.join("\n");
                let final_text = if final_text.is_empty() {
                    "Reflection recommended stopping.".to_string()
                } else {
                    final_text
                };
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: false,
                };
                return Ok((final_text, metrics));
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
