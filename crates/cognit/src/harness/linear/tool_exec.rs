use super::circuit_breaker::{CircuitBreakerStatus, ToolCallSignature};
use super::tool_budget;
use super::tool_output::{bounded_tool_result, MAX_TOOL_RESULT_BYTES};
use super::{is_context_overflow, ReActLoop, TurnMetrics};
use crate::harness::event_sink::{Event, EventSink, ToolResultEvent};

use crate::r#impl::llm::provider::{LlmProvider, StopReason, StreamChunk};
use fabric::message::{ContentBlock, Message, Role};
use fabric::ToolDefinition;
use std::future::Future;
use tracing::{debug, warn};

impl ReActLoop {
    /// Streaming variant of `run()`. Uses `llm.complete_stream()` instead of
    /// `llm.complete()` and emits granular events through `event_sink`.
    pub async fn run_streaming<L, F, Fut>(
        &mut self,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
        event_sink: &impl EventSink,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        use futures::StreamExt;

        let start = self.clock.mono_now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;

        event_sink.emit(Event::TurnStarted { iteration: 0 });

        while self.should_continue() {
            self.advance();
            event_sink.emit(Event::TurnStarted {
                iteration: self.iteration,
            });
            self.emit_loop_start(&format!("iter_{}", self.iteration));

            // Check for interrupt
            if let Some(ref flag) = self.interrupt_flag {
                if let Some(reason) = flag.take_reason() {
                    let msg = format!("[Interrupted: {:?}]", reason);
                    event_sink.emit(Event::TurnDone {
                        result: Ok(msg.clone()),
                    });
                    let metrics = TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: false,
                    };
                    return Ok((msg, metrics));
                }
            }

            // Use streaming instead of complete()
            let mut stream = match llm.complete_stream(&self.messages, tool_defs).await {
                Ok(s) => s,
                Err(e) if is_context_overflow(&e) => {
                    warn!("Context overflow detected, forcing compaction: {e}");
                    self.compressor
                        .maybe_compact(&mut self.messages, llm)
                        .await?;
                    llm.complete_stream(&self.messages, tool_defs).await?
                }
                Err(e) => return Err(e),
            };

            let mut text_parts = Vec::new();
            let mut current_text = String::new();
            let mut pending_think = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut _stop_reason = StopReason::EndTurn;

            while let Some(chunk) = stream.next().await {
                match chunk? {
                    StreamChunk::TextDelta { text } => {
                        // Flush any pending thinking content first
                        if !pending_think.is_empty() {
                            event_sink.emit(Event::TextDelta {
                                delta: pending_think.clone(),
                            });
                            pending_think.clear();
                        }
                        current_text.push_str(&text);
                        event_sink.emit(Event::TextDelta { delta: text });
                    }
                    StreamChunk::ToolUseStart { id, name } => {
                        // Flush any pending text/thinking
                        if !pending_think.is_empty() {
                            event_sink.emit(Event::TextDelta {
                                delta: pending_think.clone(),
                            });
                            pending_think.clear();
                        }
                        if !current_text.is_empty() {
                            text_parts.push(current_text.clone());
                            current_text.clear();
                        }
                        event_sink.emit(Event::ToolCallStart {
                            name: name.clone(),
                            call_id: id.clone(),
                        });
                        tool_calls.push((id, name, serde_json::Value::Null));
                    }
                    StreamChunk::ThinkingDelta { text } => {
                        // Batch thinking content — emit as single chunk when flushed
                        // to avoid per-token socket writes (which cause TUI lag).
                        pending_think.push_str(&text);
                        current_text.push_str(&text);
                    }
                    StreamChunk::ToolUseDelta { id: _, delta: _ } => {
                        // Accumulated in ToolUseComplete
                    }
                    StreamChunk::ToolUseComplete { id, input } => {
                        // Update tool_calls with correct input
                        if let Some(tc) = tool_calls.iter_mut().find(|(tid, _, _)| *tid == id) {
                            tc.2 = input.clone();
                            // Emit complete event so session tracker can record args
                            event_sink.emit(Event::ToolCallComplete {
                                call_id: id.clone(),
                                name: tc.1.clone(),
                                args: input,
                            });
                        }
                    }
                    StreamChunk::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        event_sink.emit(Event::Usage {
                            tokens_in: input_tokens,
                            tokens_out: output_tokens,
                            cache_hit_tokens: 0,
                            cache_miss_tokens: 0,
                        });
                        // Emit context window usage so TUI can display it
                        let total_estimate = self
                            .messages
                            .iter()
                            .map(|m| m.estimate_tokens())
                            .sum::<usize>() as u32;
                        event_sink.emit(Event::ContextUpdate {
                            used_tokens: total_estimate,
                            max_tokens: self.config.context_window_tokens as u32,
                        });
                    }
                    StreamChunk::Done { stop_reason: sr } => {
                        _stop_reason = sr;
                        break;
                    }
                }
            }

            // Flush any remaining thinking content
            if !pending_think.is_empty() {
                event_sink.emit(Event::TextDelta {
                    delta: pending_think,
                });
            }
            // Flush remaining text
            if !current_text.is_empty() {
                text_parts.push(current_text);
            }

            // No tool calls -> turn complete
            // Note: some models return EndTurn even when tool calls are present.
            // We must check tool_calls first — only exit if there are no tools to run.
            if tool_calls.is_empty() {
                let final_text = text_parts.join("\n");
                // Emit awareness: uncertainty from response + final response signal
                self.emit_thinking_complete("thinking", &final_text);
                self.emit_final_response("final_response");
                // Drain awareness signals and emit as events for TUI
                for (level, ctx) in self.drain_awareness_events() {
                    event_sink.emit(Event::AwarenessChanged {
                        level: level.display_name().to_string(),
                        context: ctx,
                    });
                }
                self.messages.push(Message::assistant(&final_text));
                event_sink.emit(Event::TurnDone {
                    result: Ok(final_text.clone()),
                });
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
            }

            // Has tool calls -> execute them
            let content_blocks: Vec<ContentBlock> = tool_calls
                .iter()
                .map(|(id, name, input)| ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                })
                .collect();

            self.messages.push(Message {
                role: Role::Assistant,
                content: content_blocks,
            });

            // Deferred reflection — injected after all tool results to preserve
            // OpenAI API message format (assistant(tool_use) → tool results only)
            let mut pending_reflection: Option<String> = None;

            // Collect tool results into a single combined user message.
            // Anthropic API requires ALL tool_result blocks for a given
            // assistant(tool_use) message to be in ONE subsequent user message.
            let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

            for (tool_index, (id, name, input)) in tool_calls.iter().enumerate() {
                // Defensive: skip tool calls with empty names — some
                // OpenAI-compatible providers emit malformed tool-use blocks
                // that would trip the circuit breaker.
                if name.is_empty() {
                    warn!(
                        tool_id = %id,
                        "ReActLoop streaming: skipping tool call with empty name"
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
                    event_sink.emit(Event::BudgetExceeded {
                        used: self.tool_budget.total_calls(),
                        max: self.config.max_tool_calls,
                    });
                    // The assistant tool-use message is already in history. Close every
                    // unexecuted call with an error result so the next request is
                    // structurally valid instead of poisoning the whole session.
                    // Push any already-collected results first, then the error results.
                    if !tool_result_blocks.is_empty() {
                        self.messages.push(Message {
                            role: Role::User,
                            content: std::mem::take(&mut tool_result_blocks),
                        });
                    }
                    for (pending_id, pending_name, _) in tool_calls.iter().skip(tool_index) {
                        tool_result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: pending_id.clone(),
                            content: "Tool call skipped: per-turn tool budget exhausted"
                                .to_string(),
                            is_error: true,
                        });
                        event_sink.emit(Event::ToolResult {
                            name: pending_name.clone(),
                            call_id: pending_id.clone(),
                            result: ToolResultEvent {
                                content: "Tool call skipped: per-turn tool budget exhausted"
                                    .to_string(),
                                is_error: true,
                                execution_time_ms: 0,
                            },
                        });
                    }
                    self.messages.push(Message {
                        role: Role::User,
                        content: tool_result_blocks,
                    });
                    event_sink.emit(Event::TurnDone {
                        result: Ok(msg.clone()),
                    });
                    let metrics = TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
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
                        event_sink.emit(Event::CircuitBreakerTripped {
                            reason: reason.clone(),
                        });
                        // Push any already-collected results first, then error results.
                        if !tool_result_blocks.is_empty() {
                            self.messages.push(Message {
                                role: Role::User,
                                content: std::mem::take(&mut tool_result_blocks),
                            });
                        }
                        for (pending_id, pending_name, _) in tool_calls.iter().skip(tool_index) {
                            let content =
                                format!("Tool call skipped: circuit breaker tripped: {reason}");
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: pending_id.clone(),
                                content: content.clone(),
                                is_error: true,
                            });
                            event_sink.emit(Event::ToolResult {
                                name: pending_name.clone(),
                                call_id: pending_id.clone(),
                                result: ToolResultEvent {
                                    content,
                                    is_error: true,
                                    execution_time_ms: 0,
                                },
                            });
                        }
                        self.messages.push(Message {
                            role: Role::User,
                            content: tool_result_blocks,
                        });
                        event_sink.emit(Event::TurnDone {
                            result: Ok(msg.clone()),
                        });
                        let metrics = TurnMetrics {
                            tool_calls_made,
                            tool_errors,
                            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
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

                debug!(tool = name.as_str(), "ReActLoop streaming: executing tool");
                event_sink.emit(Event::ToolDispatch {
                    name: name.clone(),
                    args: input.clone(),
                });

                let (content, is_error) = execute_tool(id, name, input).await;

                event_sink.emit(Event::ToolResult {
                    name: name.clone(),
                    call_id: id.clone(),
                    result: ToolResultEvent {
                        content: content.clone(),
                        is_error,
                        execution_time_ms: 0,
                    },
                });

                tool_calls_made += 1;
                if is_error {
                    tool_errors += 1;
                    self.consecutive_errors += 1;
                    warn!(tool = name.as_str(), "tool returned error");
                } else {
                    self.consecutive_errors = 0;
                }
                // Record call in budget tracker
                self.tool_budget.record_call(tool_budget::ToolCallRecord {
                    tool_name: name.clone(),
                    timestamp: self.clock.mono_now(),
                    success: !is_error,
                });
                // Emit awareness signal for tool completion
                self.emit_tool_call_end(name);
                // Record call in reflection engine (but don't inject yet —
                // reflections must come AFTER all tool results to preserve
                // the OpenAI API message format: assistant(tool_use) → tool results)
                let mut should_reflect = false;
                let is_timeout = is_error && content.to_lowercase().contains("timed out");
                if self.reflection_engine.record_call(is_timeout) {
                    should_reflect = true;
                }
                // The full result was emitted above for durable projection. Keep
                // only a transient bounded copy in the active model context.
                let bounded_content = bounded_tool_result(&content, MAX_TOOL_RESULT_BYTES);
                // Accumulate tool result block for combined push after loop.
                // Anthropic API requires all tool_result blocks for one assistant
                // message to be in a SINGLE subsequent user message.
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: bounded_content,
                    is_error,
                });
                // Defer reflection: collect flag, will inject after all tool results
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
                    // Emit reflection event
                    event_sink.emit(Event::Reflection {
                        summary: result.summary.clone(),
                        recommendation: format!("{:?}", result.recommendation),
                    });
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
                    .push(Message::user(format!("[Reflection]\n{}", summary)));
            }

            // Inject Dasein context after tool results for per-turn SelfField state refresh
            if let Some(ref provider) = &self.dasein_ctx_provider {
                if let Some(dasein_ctx) = provider() {
                    if !dasein_ctx.is_empty() {
                        self.messages.push(Message::user(format!(
                            "<dasein-state-update>\n{}\n</dasein-state-update>",
                            dasein_ctx
                        )));
                    }
                }
            }

            // Check if reflection recommended stopping
            if self.reflection_engine.should_stop() {
                let fallback = text_parts.join("\n");
                let fallback = if fallback.is_empty() {
                    "Reflection recommended stopping.".to_string()
                } else {
                    fallback
                };
                event_sink.emit(Event::TurnDone {
                    result: Ok(fallback.clone()),
                });
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    iterations: self.iteration,
                    completed_normally: false,
                };
                return Ok((fallback, metrics));
            }

            if self.config.compaction_enabled {
                let _ = self
                    .compressor
                    .maybe_compact(&mut self.messages, llm as &dyn LlmProvider)
                    .await;
            }
        }

        warn!(
            max = self.config.max_iterations,
            "ReActLoop streaming hit max_iterations"
        );
        let fallback = self
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
        // Drain awareness signals and emit as events for TUI
        for (level, ctx) in self.drain_awareness_events() {
            event_sink.emit(Event::AwarenessChanged {
                level: level.display_name().to_string(),
                context: ctx,
            });
        }
        event_sink.emit(Event::TurnDone {
            result: Ok(fallback.clone()),
        });
        let metrics = TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((fallback, metrics))
    }
}
