use super::{is_context_overflow, ReActLoop, TurnMetrics};
use crate::core::event_sink::{Event, EventSink, ToolResultEvent};

use base::message::{ContentBlock, Message, Role};
use base::ToolDefinition;
use cognit::r#impl::llm::provider::{LlmProvider, StopReason, StreamChunk};
use std::future::Future;
use tracing::{debug, warn};

impl ReActLoop {
    /// Streaming variant of `run()`. Uses `llm.complete_stream()` instead of
    /// `llm.complete()` and emits granular events through `event_sink`.
    pub async fn run_streaming<L, F, Fut>(
        &mut self,
        user_input: &str,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
        event_sink: &dyn EventSink,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider + ?Sized,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = (String, bool)>,
    {
        use futures::StreamExt;

        let start = std::time::Instant::now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;

        self.messages.push(Message::user(user_input));
        event_sink.emit(Event::TurnStarted);

        while self.should_continue() {
            self.advance();
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
                        elapsed_ms: start.elapsed().as_millis() as u64,
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
                    self.compressor.maybe_compact(&mut self.messages, llm).await?;
                    llm.complete_stream(&self.messages, tool_defs).await?
                }
                Err(e) => return Err(e),
            };

            let mut text_parts = Vec::new();
            let mut current_text = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;

            while let Some(chunk) = stream.next().await {
                match chunk? {
                    StreamChunk::TextDelta { text } => {
                        current_text.push_str(&text);
                        event_sink.emit(Event::TextDelta { delta: text });
                    }
                    StreamChunk::ToolUseStart { id, name } => {
                        // Flush any pending text
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
                    StreamChunk::ThinkingDelta { .. } => {
                        // Thinking content is collected in non-streaming path;
                        // in streaming mode we discard deltas for now.
                    }
                    StreamChunk::ToolUseDelta { id: _, delta: _ } => {
                        // Accumulated in ToolUseComplete
                    }
                    StreamChunk::ToolUseComplete { id, input } => {
                        // Update tool_calls with correct input
                        if let Some(tc) = tool_calls.iter_mut().find(|(tid, _, _)| *tid == id) {
                            tc.2 = input;
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
                    }
                    StreamChunk::Done { stop_reason: sr } => {
                        stop_reason = sr;
                        break;
                    }
                }
            }

            // Flush remaining text
            if !current_text.is_empty() {
                text_parts.push(current_text);
            }

            // No tool calls -> turn complete
            if tool_calls.is_empty() || matches!(stop_reason, StopReason::EndTurn) {
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
                    elapsed_ms: start.elapsed().as_millis() as u64,
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

            for (id, name, input) in &tool_calls {
                debug!(tool = name.as_str(), "ReActLoop streaming: executing tool");
                event_sink.emit(Event::ToolDispatch {
                    name: name.clone(),
                    args: input.clone(),
                });

                let (content, is_error) = execute_tool(id, name, input).await;

                event_sink.emit(Event::ToolResult {
                    name: name.clone(),
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
                // Emit awareness signal for tool completion
                self.emit_tool_call_end(name);
                self.messages
                    .push(Message::tool_result(id, &content, is_error));
            }

            if self.config.compaction_enabled {
                let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
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
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        };
        Ok((fallback, metrics))
    }
}
