use super::circuit_breaker::{CircuitBreakerStatus, ToolCallSignature};
use super::tool_budget;
use super::tool_output::{bounded_tool_result, MAX_TOOL_RESULT_BYTES};
use super::{is_context_overflow, ReActLoop, TurnMetrics};

use crate::adapters::inference::provider::LlmProvider;
use fabric::message::{ContentBlock, Message, Role};
use fabric::policy::verifier::Verdict;
use fabric::{CapabilityCall, ConsciousArbitrationMode, ToolDefinition};
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
        L: LlmProvider,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        let start = self.clock.mono_now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;
        self.verify_attempts = 0;

        self.messages.push(Message::user(user_input));

        while self.should_continue() {
            self.advance();
            self.emit_loop_start(&format!("iter_{}", self.iteration));
            let response = match llm.complete(&self.messages, tool_defs).await {
                Ok(r) => r,
                Err(e) if is_context_overflow(&e) => {
                    // A3: reactive compaction on context overflow
                    warn!("Context overflow detected, forcing compaction: {e}");
                    self.run_reactive_compaction(llm, None).await?;
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
                        // Defensive: skip tool calls with empty names — some
                        // OpenAI-compatible providers emit malformed tool-use
                        // blocks that would poison the conversation.
                        if name.is_empty() {
                            warn!(
                                tool_id = %id,
                                "ReActLoop: skipping tool call with empty name"
                            );
                            continue;
                        }
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

                // M-C: optional verification seam. Default (None) = unchanged behavior.
                if let Some(verifier) = self.verifier.clone() {
                    if self.verify_attempts < self.max_verify_attempts {
                        if let Verdict::Reject { reason } =
                            verifier.verify(&final_text, &self.messages).await
                        {
                            self.verify_attempts += 1;
                            // Record the rejected answer, then request a revision and re-loop.
                            self.messages.push(Message::assistant(&final_text));
                            self.messages.push(Message::user(format!(
                                "[verification] Your previous answer was rejected: {reason}\n\
                                 Please correct it and provide a better final answer."
                            )));
                            warn!(
                                reason = reason.as_str(),
                                "verifier rejected final answer; retrying"
                            );
                            continue;
                        }
                    }
                }

                // Emit awareness: uncertainty from response + final response signal
                self.emit_thinking_complete("thinking", &final_text);
                self.emit_final_response("final_response");
                self.messages.push(Message::assistant(&final_text));
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
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

            // Collect tool results into a single combined user message.
            // Anthropic API requires ALL tool_result blocks for one assistant
            // message to be in a SINGLE subsequent user message.
            let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

            // Execute each requested tool and feed results back.
            // Plan the batch order first.
            let calls: Vec<CapabilityCall> = tool_calls
                .iter()
                .map(|(id, name, input)| CapabilityCall {
                    operation_id: fabric::OperationId::new(),
                    process_id: fabric::ProcessId::new(),
                    name: name.clone(),
                    input: input.clone(),
                    call_id: id.clone(),
                    deadline: None,
                })
                .collect();

            let ordered_calls: Vec<&(String, String, serde_json::Value)> = if let Some(
                ref planner,
            ) = self.batch_planner
            {
                // Fail closed when an installed planner cannot establish a
                // trusted projection for every call in this batch.
                let plan = planner.plan(calls.clone()).await?;
                match plan.mode {
                    ConsciousArbitrationMode::Enforce => match plan.validate_against(&calls) {
                        Ok(()) => {
                            let mut ordered = Vec::new();
                            for id in &plan.ordered_call_ids {
                                if let Some(tc) = tool_calls.iter().find(|(tid, _, _)| tid == id) {
                                    ordered.push(tc);
                                }
                            }
                            if ordered.len() == tool_calls.len() {
                                ordered
                            } else {
                                warn!(
                                            "batch plan applied but call count mismatch; fallback to provider order"
                                        );
                                tool_calls.iter().collect()
                            }
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                mode = ?plan.mode,
                                "batch plan invalid; keeping provider order"
                            );
                            tool_calls.iter().collect()
                        }
                    },
                    ConsciousArbitrationMode::Observe => tool_calls.iter().collect(),
                }
            } else {
                tool_calls.iter().collect()
            };

            for (tool_index, (id, name, input)) in ordered_calls.iter().enumerate() {
                // Defensive: skip tool calls with empty names — some
                // OpenAI-compatible providers emit malformed tool-use blocks
                // that would trip the circuit breaker.
                if name.is_empty() {
                    warn!(
                        tool_id = %id,
                        "ReActLoop: skipping tool call with empty name"
                    );
                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: "Error: tool call has empty name — skipping".to_string(),
                        is_error: true,
                    });
                    tool_errors += 1;
                    self.consecutive_errors += 1;
                    continue;
                }

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
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: false,
                    };
                    // Push any already-collected results first, then error results.
                    if !tool_result_blocks.is_empty() {
                        self.messages.push(Message {
                            role: Role::User,
                            content: std::mem::take(&mut tool_result_blocks),
                        });
                    }
                    for (pending_id, _, _) in ordered_calls.iter().skip(tool_index) {
                        tool_result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: pending_id.clone(),
                            content: "Tool call skipped: per-turn tool budget exhausted"
                                .to_string(),
                            is_error: true,
                        });
                    }
                    self.messages.push(Message {
                        role: Role::User,
                        content: tool_result_blocks,
                    });
                    return Ok((msg, metrics));
                }

                // Check circuit breaker before executing
                let signature = ToolCallSignature::new(name, input);
                match self.circuit_breaker.check(&signature) {
                    CircuitBreakerStatus::Tripped(reason) => {
                        warn!("Circuit breaker tripped: {}", reason);
                        let msg = format!("Loop detected: {reason}. Stopping.");
                        let metrics = TurnMetrics {
                            tool_calls_made,
                            tool_errors,
                            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                            iterations: self.iteration,
                            completed_normally: false,
                        };
                        // Push any already-collected results first, then error results.
                        if !tool_result_blocks.is_empty() {
                            self.messages.push(Message {
                                role: Role::User,
                                content: std::mem::take(&mut tool_result_blocks),
                            });
                        }
                        for (pending_id, _, _) in ordered_calls.iter().skip(tool_index) {
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: pending_id.clone(),
                                content: format!(
                                    "Tool call skipped: circuit breaker tripped: {reason}"
                                ),
                                is_error: true,
                            });
                        }
                        self.messages.push(Message {
                            role: Role::User,
                            content: tool_result_blocks,
                        });
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
                    timestamp: self.clock.mono_now(),
                    success: !is_error,
                });
                // Emit awareness signal for tool completion
                self.emit_tool_call_end(name);
                // Record call in reflection engine (defer injection until after
                // all tool results to preserve OpenAI API message format)
                let mut should_reflect = false;
                let is_timeout = is_error && content.to_lowercase().contains("timed out");
                if self.reflection_engine.record_call(is_timeout) {
                    should_reflect = true;
                }
                // Keep only a transient bounded copy in the active model context.
                let bounded_content = bounded_tool_result(&content, MAX_TOOL_RESULT_BYTES);
                // Accumulate tool result block for combined push after loop.
                // Anthropic API requires all tool_result blocks for one assistant
                // message to be in a SINGLE subsequent user message.
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: bounded_content,
                    is_error,
                });
                // Defer reflection until after all tool results
                if should_reflect {
                    let ctx = crate::harness::linear::reflection::ReflectionContext {
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

            // Push combined tool result message — ALL tool_results for the
            // preceding assistant(tool_use) message MUST be in ONE user message
            // for the Anthropic API (tool_result blocks immediately after tool_use).
            if !tool_result_blocks.is_empty() {
                self.messages.push(Message {
                    role: Role::User,
                    content: tool_result_blocks,
                });
            }

            // Inject reflection AFTER all tool results to preserve API message format
            if let Some(summary) = pending_reflection.take() {
                self.messages
                    .push(Message::user(format!("[Reflection]\n{summary}")));
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
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    iterations: self.iteration,
                    completed_normally: false,
                };
                return Ok((final_text, metrics));
            }

            // A2: proactive compaction after pushing tool results
            if self.config.compaction_enabled {
                self.run_proactive_compaction(llm, None).await;
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
            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((final_text, metrics))
    }
}
