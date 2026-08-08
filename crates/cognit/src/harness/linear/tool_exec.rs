use super::circuit_breaker::{CircuitBreakerStatus, ToolCallSignature};
use super::completion::FinalizationDecision;
use super::tool_budget;
use super::tool_output::{bounded_tool_result, per_result_budget};
use super::{is_context_overflow, ReActLoop, TurnMetrics};
use crate::harness::event_sink::{Event, EventSink, ToolResultEvent};

use crate::adapters::inference::provider::{LlmProvider, StopReason, StreamChunk};
use crate::core::{
    AgentRuntimeId, CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState,
    CognitiveWorkPhase, CompletionGateMode, EvidenceId, EvidenceLevel, EvidenceLocator,
    EvidenceRecord, EvidenceSource, EvidenceSubject, RequiredAction, TerminalStatus,
};
use crate::inference::{classify_error, ErrorClass};
use fabric::message::{ContentBlock, Message, Role};
use fabric::{CapabilityCall, ConsciousArbitrationMode, ToolDefinition};
use std::future::Future;
use tracing::{debug, warn};

impl ReActLoop {
    /// Streaming variant of `run()`. Uses `llm.complete_stream()` instead of
    /// `llm.complete()` and emits granular events through `event_sink`.
    pub async fn run_streaming<L, F, Fut, O, D, DFut>(
        &mut self,
        llm: &L,
        tool_defs: &[ToolDefinition],
        execute_tool: F,
        drain_interjections: D,
        event_sink: &impl EventSink,
    ) -> anyhow::Result<(String, TurnMetrics)>
    where
        L: LlmProvider,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: Future<Output = O>,
        O: Into<ToolResultEvent>,
        D: Fn() -> DFut,
        DFut: Future<Output = anyhow::Result<Vec<String>>>,
    {
        use futures::StreamExt;

        let start = self.clock.mono_now();
        let mut tool_calls_made: usize = 0;
        let mut tool_errors: usize = 0;
        let mut provider_retries = 0_u64;
        let mut visible_tool_defs = tool_defs.to_vec();
        self.verify_attempts = 0;

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
                    let msg = format!("[Interrupted: {reason:?}]");
                    event_sink.emit(Event::TurnDone {
                        result: Ok(msg.clone()),
                    });
                    let metrics = TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        provider_retries,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: false,
                        stop: fabric::TurnStop::Cancelled,
                    };
                    return Ok((msg, metrics));
                }
            }

            // Use streaming instead of complete()
            let mut transient_attempt = 0_u32;
            let mut stream = loop {
                match llm
                    .complete_stream(&self.messages, &visible_tool_defs)
                    .await
                {
                    Ok(stream) => break stream,
                    Err(e) if is_context_overflow(&e) => {
                        warn!("Context overflow detected, forcing compaction: {e}");
                        self.run_reactive_compaction(llm, Some(event_sink)).await?;
                    }
                    Err(e)
                        if classify_error(&e) == ErrorClass::Transient && transient_attempt < 4 =>
                    {
                        let backoff_ms = streaming_retry_delay_ms(&e, transient_attempt);
                        transient_attempt += 1;
                        provider_retries = provider_retries.saturating_add(1);
                        warn!(
                            attempt = transient_attempt,
                            backoff_ms, error = %e,
                            "Streaming inference unavailable; retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    }
                    Err(e) => return Err(e),
                }
            };

            let mut text_parts = Vec::new();
            let mut current_text = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut _stop_reason = StopReason::EndTurn;

            while let Some(chunk) = stream.next().await {
                match chunk? {
                    StreamChunk::TextDelta { text } => {
                        current_text.push_str(&text);
                        event_sink.emit(Event::TextDelta { delta: text });
                    }
                    StreamChunk::ToolUseStart { id, name } => {
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
                    StreamChunk::ThinkingDelta { text: _ } => {
                        // Thinking is internal model state. Keep it out of both
                        // the visible text stream and the persisted answer.
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
                    StreamChunk::Usage { usage } => {
                        // Provider input usage is the authoritative occupancy
                        // of the prompt actually submitted for this round. The
                        // previous local message estimate omitted tool schemas
                        // and the current assistant/tool exchange, so the TUI
                        // could report a materially incorrect context load.
                        let used_tokens = usage
                            .total_input_tokens
                            .map(|tokens| u32::try_from(tokens).unwrap_or(u32::MAX))
                            .unwrap_or_else(|| {
                                self.messages
                                    .iter()
                                    .map(|message| message.estimate_tokens())
                                    .sum::<usize>()
                                    .try_into()
                                    .unwrap_or(u32::MAX)
                            });
                        self.turn_input_tokens = self
                            .turn_input_tokens
                            .saturating_add(usage.total_input_tokens.unwrap_or(0));
                        event_sink.emit(Event::Usage { usage });
                        event_sink.emit(Event::ContextUpdate {
                            used_tokens,
                            max_tokens: self
                                .config
                                .context_window_tokens
                                .try_into()
                                .unwrap_or(u32::MAX),
                        });
                    }
                    StreamChunk::Done { stop_reason: sr } => {
                        _stop_reason = sr;
                        break;
                    }
                }
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
                let final_text = match self
                    .finalize_candidate(final_text, &drain_interjections)
                    .await?
                {
                    FinalizationDecision::ContinueWithInterjections {
                        assistant_text,
                        interjections,
                    } => {
                        if let Some(text) = assistant_text {
                            self.messages.push(Message::assistant(&text));
                        }
                        self.messages
                            .extend(interjections.into_iter().map(Message::user));
                        continue;
                    }
                    FinalizationDecision::ContinueAfterRejection => continue,
                    FinalizationDecision::Incomplete { message } => {
                        event_sink.emit(Event::TurnDone {
                            result: Err(message.clone()),
                        });
                        let metrics = TurnMetrics {
                            tool_calls_made,
                            tool_errors,
                            provider_retries,
                            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                            iterations: self.iteration,
                            completed_normally: false,
                            stop: fabric::TurnStop::Blocked,
                        };
                        return Ok((message, metrics));
                    }
                    FinalizationDecision::Accept { final_text } => final_text,
                };
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
                    provider_retries,
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    iterations: self.iteration,
                    completed_normally: true,
                    stop: fabric::TurnStop::Completed,
                };
                return Ok((final_text, metrics));
            }

            // Has tool calls -> execute them
            // Collect into CapabilityCall slice so the batch planner can operate on them.
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

            // Plan the batch order. Only apply the ordered permutation when
            // the plan is Enforce and is a valid exact permutation; otherwise
            // keep provider order (the order the LLM emitted).
            let ordered_calls: Vec<&(String, String, serde_json::Value)> = if let Some(
                ref planner,
            ) = self.batch_planner
            {
                // A configured production planner is a trust boundary. Its
                // rejection must stop the batch rather than silently execute
                // unprojected calls in provider order.
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
                    ConsciousArbitrationMode::Observe => {
                        // Observe mode: always keep provider order.
                        tool_calls.iter().collect()
                    }
                }
            } else {
                tool_calls.iter().collect()
            };

            let content_blocks: Vec<ContentBlock> = ordered_calls
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

            if super::should_close_exploration(
                self.iteration,
                self.turn_input_tokens,
                self.config.context_window_tokens,
                self.repository_context_seen,
                self.broad_discovery_batches,
                ordered_calls
                    .iter()
                    .map(|(_, name, input)| (name.as_str(), input)),
            ) {
                let budget =
                    super::exploration_input_token_budget(self.config.context_window_tokens);
                let content = format!(
                    "Broad-scanning budget reached ({} / {} input tokens). \
                     Stop repository scanning and answer now from the evidence \
                     you have ACTUALLY \
                     gathered from tool output. Do not present unverified inferences \
                     as fact: mark any claim you could not confirm as \"(unverified)\" \
                     and say what you would need to check to confirm it.",
                    self.turn_input_tokens, budget
                );
                let results = exploration_budget_results(&ordered_calls, &content, event_sink);
                self.messages.push(Message {
                    role: Role::User,
                    content: results,
                });
                self.messages.push(Message::user(
                    "[synthesis] Tools are disabled for this final pass. Produce a substantive \
                     answer using only the repository evidence already returned. Clearly mark \
                     any unverified claim."
                        .to_string(),
                ));
                let response = llm.complete(&self.messages, &[]).await?;
                let final_text = response
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let final_text = if final_text.trim().is_empty() {
                    "Repository exploration was closed after sufficient evidence, but the final synthesis returned no text."
                        .to_string()
                } else {
                    final_text
                };
                event_sink.emit(Event::TextDelta {
                    delta: final_text.clone(),
                });
                event_sink.emit(Event::TurnDone {
                    result: Ok(final_text.clone()),
                });
                return Ok((
                    final_text,
                    TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        provider_retries,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: true,
                        stop: fabric::TurnStop::Completed,
                    },
                ));
            }

            // Deferred reflection — injected after all tool results to preserve
            // OpenAI API message format (assistant(tool_use) → tool results only)
            let mut pending_reflection: Option<String> = None;

            // Collect tool results into a single combined user message.
            // Anthropic API requires ALL tool_result blocks for a given
            // assistant(tool_use) message to be in ONE subsequent user message.
            let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
            let mut clarification_requested: Option<String> = None;

            let result_budget = per_result_budget(ordered_calls.len());
            for (tool_index, (id, name, input)) in ordered_calls.iter().enumerate() {
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
                    for (pending_id, pending_name, _) in ordered_calls.iter().skip(tool_index) {
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
                                patch_delta: None,
                                activated_tool_definitions: Vec::new(),
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
                        provider_retries,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: false,
                        stop: fabric::TurnStop::Blocked,
                    };
                    return Ok((msg, metrics));
                }

                // Check circuit breaker before executing
                let signature = ToolCallSignature::new(name, input);
                match self.circuit_breaker.check(&signature) {
                    CircuitBreakerStatus::Tripped(reason) => {
                        warn!("Circuit breaker tripped: {}", reason);
                        let msg = format!("Loop detected: {reason}. Stopping.");
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
                        for (pending_id, pending_name, _) in ordered_calls.iter().skip(tool_index) {
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
                                    patch_delta: None,
                                    activated_tool_definitions: Vec::new(),
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
                            provider_retries,
                            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                            iterations: self.iteration,
                            completed_normally: false,
                            stop: fabric::TurnStop::Blocked,
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

                let tool_result: ToolResultEvent = execute_tool(id, name, input).await.into();
                let content = tool_result.content.clone();
                let is_error = tool_result.is_error;
                let previous_visible_count = visible_tool_defs.len();
                for definition in &tool_result.activated_tool_definitions {
                    if !visible_tool_defs
                        .iter()
                        .any(|visible| visible.name == definition.name)
                    {
                        visible_tool_defs.push(definition.clone());
                    }
                }
                if visible_tool_defs.len() != previous_visible_count {
                    tracing::info!(
                        tool = name.as_str(),
                        activated = visible_tool_defs.len() - previous_visible_count,
                        visible_tools = visible_tool_defs.len(),
                        "Expanded model-visible tool projection"
                    );
                }

                self.evidence_ledger.record(EvidenceRecord {
                    id: EvidenceId(format!("tool:{id}")),
                    subject: EvidenceSubject::ToolInvocation {
                        tool_name: name.clone(),
                    },
                    source: EvidenceSource::Tool { name: name.clone() },
                    level: EvidenceLevel::Observed,
                    terminal_status: if is_error {
                        TerminalStatus::Failed
                    } else {
                        TerminalStatus::Succeeded
                    },
                    locator: EvidenceLocator::DurableReceipt {
                        receipt_id: id.clone(),
                    },
                    digest: None,
                });
                if let Some((agent_id, runtime)) = agent_spawn_observation(name, &content, is_error)
                {
                    self.spawned_agents.insert(agent_id, runtime);
                }
                if let Some((runtime, terminal_status)) =
                    agent_terminal_evidence(name, &content, is_error, &self.spawned_agents)
                {
                    self.evidence_ledger.record(EvidenceRecord {
                        id: EvidenceId(format!("agent:{id}")),
                        subject: EvidenceSubject::AgentInvocation {
                            runtime: runtime.clone(),
                        },
                        source: EvidenceSource::AgentRuntime { runtime },
                        level: EvidenceLevel::RuntimeVerified,
                        terminal_status,
                        locator: EvidenceLocator::DurableReceipt {
                            receipt_id: id.clone(),
                        },
                        digest: None,
                    });
                }
                self.observe_change_transaction(name, id, &content, is_error);
                self.observe_managed_command(name, id, &content, is_error);
                self.observe_evaluation_validation(name, id, &content, is_error);
                if name == "request_user_input" && !is_error {
                    clarification_requested = clarification_question(&content);
                }

                event_sink.emit(Event::ToolResult {
                    name: name.clone(),
                    call_id: id.clone(),
                    result: tool_result,
                });

                tool_calls_made += 1;
                if is_error {
                    tool_errors += 1;
                    self.consecutive_errors += 1;
                    warn!(tool = name.as_str(), "tool returned error");
                } else {
                    self.consecutive_errors = 0;
                    if name == "repo_inspect" {
                        self.repository_context_seen = true;
                    }
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
                let bounded_content = bounded_tool_result(&content, result_budget);
                // Accumulate tool result block for combined push after loop.
                // Anthropic API requires all tool_result blocks for one assistant
                // message to be in a SINGLE subsequent user message.
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: bounded_content,
                    is_error,
                });
                if let Some(outcome) = authoritative_policy_block(&content, is_error) {
                    // A host policy denial is an authoritative terminal boundary,
                    // not model feedback that may be worked around. Close every
                    // remaining tool-use block without dispatching it so provider
                    // history remains structurally valid, then settle the turn as
                    // blocked without another inference request.
                    for (pending_id, pending_name, _) in ordered_calls.iter().skip(tool_index + 1) {
                        let skipped =
                            "Tool call skipped: an earlier call was blocked by host policy";
                        tool_result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: pending_id.clone(),
                            content: skipped.to_string(),
                            is_error: true,
                        });
                        event_sink.emit(Event::ToolResult {
                            name: pending_name.clone(),
                            call_id: pending_id.clone(),
                            result: ToolResultEvent {
                                content: skipped.to_string(),
                                is_error: true,
                                execution_time_ms: 0,
                                patch_delta: None,
                                activated_tool_definitions: Vec::new(),
                            },
                        });
                    }
                    self.messages.push(Message {
                        role: Role::User,
                        content: tool_result_blocks,
                    });
                    event_sink.emit(Event::TurnDone {
                        result: Ok(outcome.clone()),
                    });
                    return Ok((
                        outcome,
                        TurnMetrics {
                            tool_calls_made,
                            tool_errors,
                            provider_retries,
                            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                            iterations: self.iteration,
                            completed_normally: false,
                            stop: fabric::TurnStop::Blocked,
                        },
                    ));
                }
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
            // Only broad-discovery batches (contains at least one repository-wide
            // scan) count toward the cut-off allowance. Scoped/exact discovery
            // calls do not consume the counter. Any read-only tool (including
            // file_read) mixed with a broad scan does not hide the scan from
            // the counter; any non-inspection tool prevents both increment and
            // closure via the all_inspection gate in should_close_exploration.
            if self.repository_context_seen
                && !ordered_calls.is_empty()
                && ordered_calls
                    .iter()
                    .any(|(_, name, input)| super::is_broad_discovery(name, input))
                && ordered_calls.iter().all(|(_, name, _)| {
                    matches!(name.as_str(), "glob" | "grep" | "file_search" | "file_read")
                })
            {
                self.broad_discovery_batches = self.broad_discovery_batches.saturating_add(1);
            }
            // Inject reflection AFTER all tool results to preserve API message format
            if let Some(summary) = pending_reflection.take() {
                self.messages
                    .push(Message::user(format!("[Reflection]\n{summary}")));
            }

            // Recovery nudge: repeated tool failures usually mean the current
            // approach is not working. Prompt the model to reassess and change
            // course rather than looping on the same failing call.
            if self.consecutive_errors >= super::REPLAN_ON_CONSECUTIVE_ERRORS {
                self.messages.push(Message::user(format!(
                    "[recover] {} tool calls have failed in a row. Do not repeat the same \
                     call. Reassess: is this approach viable? Try a different tool or \
                     different parameters, or if the goal is blocked, state what is blocking \
                     it and stop.",
                    self.consecutive_errors
                )));
                self.consecutive_errors = 0;
            }

            // Inject Dasein context after tool results for per-turn SelfField state refresh
            if let Some(ref provider) = &self.dasein_ctx_provider {
                if let Some(dasein_ctx) = provider() {
                    if !dasein_ctx.is_empty() {
                        self.messages.push(Message::user(format!(
                            "<dasein-state-update>\n{dasein_ctx}\n</dasein-state-update>"
                        )));
                    }
                }
            }

            // G3 safe point: all tool results have been absorbed and no tool
            // side effect or settlement is in flight. Keep each interjection
            // as an independent synthetic user message in FIFO order.
            self.messages
                .extend(drain_interjections().await?.into_iter().map(Message::user));

            if let Some(question) = clarification_requested {
                let outcome = format!("Waiting for user clarification: {question}");
                event_sink.emit(Event::TurnDone {
                    result: Ok(outcome.clone()),
                });
                return Ok((
                    outcome,
                    TurnMetrics {
                        tool_calls_made,
                        tool_errors,
                        provider_retries,
                        elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                        iterations: self.iteration,
                        completed_normally: false,
                        stop: fabric::TurnStop::Blocked,
                    },
                ));
            }

            // Check if reflection recommended stopping.
            if self.reflection_engine.should_stop() {
                let mut fallback = text_parts.join("\n");
                let mut synthesized_for_display = None;
                // Reflection halts the tool loop, but must not surface an empty
                // or stub answer. When no substantive answer exists yet, force
                // ONE final tool-free synthesis pass over the gathered evidence.
                if fallback.trim().chars().count() < super::MIN_SUBSTANTIVE_ANSWER_CHARS {
                    self.messages.push(Message::user(
                        "[synthesis] You must stop calling tools now. Provide your best \
                         final answer based only on the evidence you have actually gathered. \
                         Mark any claim you could not verify from tool output as \"(unverified)\"."
                            .to_string(),
                    ));
                    let no_tools: &[ToolDefinition] = &[];
                    match llm.complete(&self.messages, no_tools).await {
                        Ok(resp) => {
                            let synth: String = resp
                                .content
                                .iter()
                                .filter_map(|block| match block {
                                    ContentBlock::Text { text } => Some(text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !synth.trim().is_empty() {
                                fallback = synth.clone();
                                synthesized_for_display = Some(synth);
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "forced synthesis after reflection-stop failed (streaming)");
                        }
                    }
                }
                if fallback.trim().is_empty() {
                    fallback = "Reflection recommended stopping; no answer could be synthesized."
                        .to_string();
                    synthesized_for_display = Some(fallback.clone());
                }
                // `complete()` does not produce streaming deltas. The TUI renders
                // assistant content from TextDelta events rather than TurnDone's
                // terminal receipt, so publish the forced synthesis exactly once.
                if let Some(synth) = synthesized_for_display {
                    event_sink.emit(Event::TextDelta { delta: synth });
                }
                event_sink.emit(Event::TurnDone {
                    result: Ok(fallback.clone()),
                });
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    provider_retries,
                    elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
                    iterations: self.iteration,
                    completed_normally: false,
                    stop: fabric::TurnStop::Blocked,
                };
                return Ok((fallback, metrics));
            }

            if self.config.compaction_enabled {
                self.run_proactive_compaction(llm as &dyn LlmProvider, Some(event_sink))
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
            provider_retries,
            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
            iterations: self.iteration,
            completed_normally: false,
            stop: fabric::TurnStop::Blocked,
        };
        Ok((fallback, metrics))
    }
}

fn authoritative_policy_block(content: &str, is_error: bool) -> Option<String> {
    if !is_error {
        return None;
    }
    let normalized = content
        .trim()
        .strip_prefix("[ERROR] ")
        .unwrap_or_else(|| content.trim());
    if normalized.starts_with("Policy denied:") || normalized.starts_with("Escalate to human:") {
        Some(format!(
            "Tool execution blocked by host policy: {normalized}"
        ))
    } else {
        None
    }
}

enum ChangeTransactionObservation {
    Applied {
        transaction_id: String,
        workspace_version: String,
        requires_validation: bool,
    },
    DiffReviewed {
        transaction_id: String,
        workspace_version: String,
        artifact_ref: String,
        validated_without_command: bool,
    },
    Validated {
        transaction_id: String,
        workspace_version: String,
        artifact_ref: Option<String>,
    },
}

impl ReActLoop {
    fn observe_managed_command(
        &mut self,
        capability: &str,
        call_id: &str,
        content: &str,
        is_error: bool,
    ) {
        if !matches!(
            capability,
            "exec_command" | "validation_run" | "write_stdin"
        ) {
            return;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(content) else {
            return;
        };
        let Some(session_id) = payload.get("session_id").and_then(|value| value.as_str()) else {
            return;
        };
        let terminal = payload.get("terminal").filter(|value| !value.is_null());
        if terminal.is_none() {
            if is_error {
                return;
            }
            if self.cognitive_state.is_none() {
                self.cognitive_state =
                    Some(CognitiveTurnState::from_contract(CognitiveTaskContract {
                        objective: "observe the authoritative managed-command result".into(),
                        task_kind: CognitiveTaskKind::General,
                        required_actions: Vec::new(),
                        deliverables: Vec::new(),
                        validation_requirements: Vec::new(),
                    }));
            }
            if let Some(state) = self.cognitive_state.as_mut() {
                state.require_action(RequiredAction::ObserveCommandSession {
                    session_id: session_id.into(),
                });
            }
            self.completion_gate_mode = CompletionGateMode::Enforce;
            return;
        }
        let terminal = terminal.expect("checked above");
        let succeeded = terminal.get("status").and_then(|value| value.as_str()) == Some("exited")
            && terminal.get("exit_code").and_then(|value| value.as_i64()) == Some(0)
            && !is_error;
        let locator = payload
            .get("output_artifact_ref")
            .and_then(|value| value.as_str())
            .map(|artifact_id| EvidenceLocator::Artifact {
                artifact_id: artifact_id.into(),
            })
            .unwrap_or_else(|| EvidenceLocator::DurableReceipt {
                receipt_id: call_id.into(),
            });
        self.evidence_ledger.record(EvidenceRecord {
            id: EvidenceId(format!("command-terminal:{call_id}")),
            subject: EvidenceSubject::CommandSessionTerminal {
                session_id: session_id.into(),
            },
            source: EvidenceSource::Tool {
                name: capability.into(),
            },
            level: EvidenceLevel::DeterministicallyVerified,
            terminal_status: if succeeded {
                TerminalStatus::Succeeded
            } else {
                TerminalStatus::Failed
            },
            locator,
            digest: payload
                .get("output_artifact_ref")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
    }

    fn observe_evaluation_validation(
        &mut self,
        capability: &str,
        call_id: &str,
        content: &str,
        is_error: bool,
    ) {
        if capability != "validation_run" {
            return;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(content) else {
            return;
        };
        let Some(terminal) = payload.get("terminal").filter(|value| !value.is_null()) else {
            return;
        };
        let requirement_id = self.cognitive_state.as_ref().and_then(|state| {
            state
                .contract
                .validation_requirements
                .iter()
                .find_map(|item| {
                    if !item
                        .id
                        .starts_with("evaluation:evidence:verification_result:")
                    {
                        return None;
                    }
                    let subject = EvidenceSubject::Validation {
                        requirement_id: item.id.clone(),
                    };
                    self.evidence_ledger
                        .successful_for(&subject)
                        .is_none()
                        .then(|| item.id.clone())
                })
        });
        let Some(requirement_id) = requirement_id else {
            return;
        };
        let succeeded = terminal.get("status").and_then(|value| value.as_str()) == Some("exited")
            && terminal.get("exit_code").and_then(|value| value.as_i64()) == Some(0)
            && !is_error;
        let locator = payload
            .get("output_artifact_ref")
            .and_then(|value| value.as_str())
            .map(|artifact_id| EvidenceLocator::Artifact {
                artifact_id: artifact_id.into(),
            })
            .unwrap_or_else(|| EvidenceLocator::DurableReceipt {
                receipt_id: call_id.into(),
            });
        self.evidence_ledger.record(EvidenceRecord {
            id: EvidenceId(format!("evaluation-validation:{call_id}:{requirement_id}")),
            subject: EvidenceSubject::Validation { requirement_id },
            source: EvidenceSource::Validation {
                name: capability.into(),
            },
            level: EvidenceLevel::DeterministicallyVerified,
            terminal_status: if succeeded {
                TerminalStatus::Succeeded
            } else {
                TerminalStatus::Failed
            },
            locator,
            digest: payload
                .get("output_artifact_ref")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
    }

    fn observe_change_transaction(
        &mut self,
        capability: &str,
        call_id: &str,
        content: &str,
        is_error: bool,
    ) {
        let Some(observation) = change_transaction_observation(capability, content, is_error)
        else {
            return;
        };
        match observation {
            ChangeTransactionObservation::Applied {
                transaction_id,
                workspace_version,
                requires_validation,
            } => {
                if self.cognitive_state.is_none() {
                    self.cognitive_state =
                        Some(CognitiveTurnState::from_contract(CognitiveTaskContract {
                            objective: "complete a version-bound code change".into(),
                            task_kind: CognitiveTaskKind::CodeChange,
                            required_actions: Vec::new(),
                            deliverables: Vec::new(),
                            validation_requirements: Vec::new(),
                        }));
                }
                if let Some(state) = self.cognitive_state.as_mut() {
                    state.phase = CognitiveWorkPhase::Execute;
                    state.require_current_change_version(
                        &transaction_id,
                        &workspace_version,
                        requires_validation,
                    );
                    // The model must produce review and validation evidence,
                    // but cannot settle its own mutation. Acceptance/repair is
                    // a later Host review action and is intentionally absent
                    // from the production model-tool registry.
                }
                self.completion_gate_mode = CompletionGateMode::Enforce;
            }
            ChangeTransactionObservation::DiffReviewed {
                transaction_id,
                workspace_version,
                artifact_ref,
                validated_without_command,
            } => {
                self.evidence_ledger.record(EvidenceRecord {
                    id: EvidenceId(format!("change-diff:{call_id}")),
                    subject: EvidenceSubject::ChangeDiffReview {
                        transaction_id: transaction_id.clone(),
                        workspace_version: workspace_version.clone(),
                    },
                    source: EvidenceSource::Tool {
                        name: capability.into(),
                    },
                    level: EvidenceLevel::DeterministicallyVerified,
                    terminal_status: TerminalStatus::Succeeded,
                    locator: EvidenceLocator::Artifact {
                        artifact_id: artifact_ref,
                    },
                    digest: Some(workspace_version.clone()),
                });
                if let Some(state) = self.cognitive_state.as_mut() {
                    state.phase = CognitiveWorkPhase::Verify;
                }
                if validated_without_command {
                    self.evidence_ledger.record(EvidenceRecord {
                        id: EvidenceId(format!("change-validation:{call_id}")),
                        subject: EvidenceSubject::ChangeValidation {
                            transaction_id,
                            workspace_version: workspace_version.clone(),
                        },
                        source: EvidenceSource::HostRuntime,
                        level: EvidenceLevel::DeterministicallyVerified,
                        terminal_status: TerminalStatus::Succeeded,
                        locator: EvidenceLocator::DurableReceipt {
                            receipt_id: call_id.into(),
                        },
                        digest: Some(workspace_version),
                    });
                }
            }
            ChangeTransactionObservation::Validated {
                transaction_id,
                workspace_version,
                artifact_ref,
            } => {
                self.evidence_ledger.record(EvidenceRecord {
                    id: EvidenceId(format!("change-validation:{call_id}")),
                    subject: EvidenceSubject::ChangeValidation {
                        transaction_id,
                        workspace_version: workspace_version.clone(),
                    },
                    source: EvidenceSource::Validation {
                        name: capability.into(),
                    },
                    level: EvidenceLevel::DeterministicallyVerified,
                    terminal_status: TerminalStatus::Succeeded,
                    locator: artifact_ref.map_or_else(
                        || EvidenceLocator::DurableReceipt {
                            receipt_id: call_id.into(),
                        },
                        |artifact_id| EvidenceLocator::Artifact { artifact_id },
                    ),
                    digest: Some(workspace_version),
                });
                if let Some(state) = self.cognitive_state.as_mut() {
                    state.phase = CognitiveWorkPhase::Synthesize;
                }
            }
        }
    }
}

fn change_transaction_observation(
    capability: &str,
    content: &str,
    is_error: bool,
) -> Option<ChangeTransactionObservation> {
    if is_error {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(content).ok()?;
    if matches!(capability, "apply_patch" | "file_write")
        && matches!(
            payload.get("kind")?.as_str()?,
            "apply_patch_receipt" | "file_write_receipt"
        )
    {
        return Some(ChangeTransactionObservation::Applied {
            transaction_id: payload.get("transaction_id")?.as_str()?.into(),
            workspace_version: payload.get("resulting_workspace_version")?.as_str()?.into(),
            requires_validation: payload
                .get("validation_plan")
                .and_then(|value| value.as_array())
                .map(|steps| {
                    steps.iter().any(|step| {
                        step.get("required")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(true)
                    })
                })
                .unwrap_or(true),
        });
    }
    if capability == "git_diff" && payload.get("kind")?.as_str()? == "change_diff_receipt" {
        return Some(ChangeTransactionObservation::DiffReviewed {
            transaction_id: payload.get("transaction_id")?.as_str()?.into(),
            workspace_version: payload.get("workspace_version")?.as_str()?.into(),
            artifact_ref: payload.get("diff_artifact_ref")?.as_str()?.into(),
            validated_without_command: payload
                .get("transaction_phase")
                .and_then(|value| value.as_str())
                == Some("validated"),
        });
    }
    if matches!(capability, "exec_command" | "write_stdin") {
        if let Some(transaction) = payload.get("change_transaction") {
            if transaction.get("phase")?.as_str()? == "applied" {
                return Some(ChangeTransactionObservation::Applied {
                    transaction_id: transaction.get("transaction_id")?.as_str()?.into(),
                    workspace_version: transaction.get("current")?.get("digest")?.as_str()?.into(),
                    requires_validation: transaction
                        .get("validation_plan")
                        .and_then(|value| value.as_array())
                        .map(|steps| {
                            steps.iter().any(|step| {
                                step.get("required")
                                    .and_then(|value| value.as_bool())
                                    .unwrap_or(true)
                            })
                        })
                        .unwrap_or(true),
                });
            }
        }
    }
    if !matches!(capability, "validation_run" | "write_stdin") {
        return None;
    }
    let transaction = payload.get("change_transaction")?;
    if transaction.get("phase")?.as_str()? != "validated" {
        return None;
    }
    let receipt = transaction.get("validation_receipts")?.as_array()?.last()?;
    Some(ChangeTransactionObservation::Validated {
        transaction_id: transaction.get("transaction_id")?.as_str()?.into(),
        workspace_version: receipt.get("workspace_version")?.as_str()?.into(),
        artifact_ref: receipt
            .get("output_ref")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

fn clarification_question(content: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(content).ok()?;
    (payload.get("status")?.as_str()? == "blocked")
        .then(|| payload.get("question")?.as_str().map(str::to_owned))?
}

#[cfg(test)]
mod change_transaction_tests {
    use super::*;
    use crate::adapters::inference::provider::LlmProvider;
    use crate::core::{Obligation, ProgressAuditor, ProgressDecision};
    use crate::harness::linear::{CompactorTrait, HarnessConfig};
    use fabric::message::Message;
    use std::pin::Pin;

    struct NoopCompressor;
    impl CompactorTrait for NoopCompressor {
        fn maybe_compact<'a>(
            &'a mut self,
            _messages: &'a mut Vec<Message>,
            _llm: &'a dyn LlmProvider,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
            Box::pin(async { Ok(false) })
        }

        fn force_compact<'a>(
            &'a mut self,
            _messages: &'a mut Vec<Message>,
            _llm: &'a dyn LlmProvider,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
            Box::pin(async { Ok(false) })
        }
    }

    #[test]
    fn version_bound_change_completes_after_diff_review_and_validation() {
        let mut loop_state = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
        loop_state.observe_change_transaction(
            "apply_patch",
            "apply",
            r#"{"kind":"apply_patch_receipt","transaction_id":"tx","resulting_workspace_version":"v1"}"#,
            false,
        );
        assert!(matches!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Continue { ref missing }
                if missing == &vec![Obligation::RequiredAction(
                    RequiredAction::ReviewChange {
                        transaction_id: "tx".into(),
                        workspace_version: "v1".into(),
                    }
                )]
        ));

        loop_state.observe_change_transaction(
            "git_diff",
            "diff",
            r#"{"kind":"change_diff_receipt","transaction_id":"tx","workspace_version":"v1","diff_artifact_ref":"artifact://sha256/diff"}"#,
            false,
        );
        loop_state.observe_change_transaction(
            "validation_run",
            "validation",
            r#"{"change_transaction":{"transaction_id":"tx","phase":"validated","validation_receipts":[{"workspace_version":"v1","output_ref":"artifact://sha256/test"}]}}"#,
            false,
        );
        assert_eq!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Complete
        );
    }

    #[test]
    fn validation_free_change_completes_after_host_derived_review() {
        let mut loop_state = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
        loop_state.observe_change_transaction(
            "file_write",
            "write",
            r#"{"kind":"file_write_receipt","transaction_id":"tx","resulting_workspace_version":"v1","validation_plan":[]}"#,
            false,
        );
        assert!(matches!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Continue { ref missing } if missing.len() == 1
        ));

        loop_state.observe_change_transaction(
            "git_diff",
            "diff",
            r#"{"kind":"change_diff_receipt","transaction_id":"tx","workspace_version":"v1","diff_artifact_ref":"artifact://sha256/diff","transaction_phase":"validated"}"#,
            false,
        );
        assert_eq!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Complete
        );
    }

    #[test]
    fn stale_or_failed_receipts_do_not_satisfy_change_obligations() {
        let mut loop_state = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
        loop_state.observe_change_transaction(
            "file_write",
            "write",
            r#"{"kind":"file_write_receipt","transaction_id":"tx","resulting_workspace_version":"v2"}"#,
            false,
        );
        loop_state.observe_change_transaction(
            "git_diff",
            "stale-diff",
            r#"{"kind":"change_diff_receipt","transaction_id":"tx","workspace_version":"v1","diff_artifact_ref":"artifact://sha256/stale"}"#,
            false,
        );
        loop_state.observe_change_transaction(
            "validation_run",
            "failed",
            r#"{"change_transaction":{"transaction_id":"tx","phase":"validated","validation_receipts":[{"workspace_version":"v2"}]}}"#,
            true,
        );
        assert!(matches!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Continue { ref missing }
                if missing == &vec![Obligation::RequiredAction(
                    RequiredAction::ReviewChange {
                        transaction_id: "tx".into(),
                        workspace_version: "v2".into(),
                    }
                )]
        ));
    }

    #[test]
    fn host_policy_denial_is_a_typed_terminal_boundary() {
        assert_eq!(
            authoritative_policy_block("Policy denied: workspace is read-only", true),
            Some(
                "Tool execution blocked by host policy: Policy denied: workspace is read-only"
                    .into()
            )
        );
        assert_eq!(
            authoritative_policy_block("[ERROR] Escalate to human: approval required", true),
            Some(
                "Tool execution blocked by host policy: Escalate to human: approval required"
                    .into()
            )
        );
        assert_eq!(
            authoritative_policy_block("Policy denied: diagnostic text", false),
            None
        );
        assert_eq!(
            authoritative_policy_block("ordinary tool failure", true),
            None
        );
    }

    #[test]
    fn running_managed_command_blocks_completion_until_terminal_snapshot() {
        let mut loop_state = ReActLoop::new(HarnessConfig::default(), Box::new(NoopCompressor));
        loop_state.observe_managed_command(
            "exec_command",
            "start",
            r#"{"session_id":"command-1","terminal":null}"#,
            false,
        );
        assert!(matches!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Continue { ref missing } if missing.len() == 1
        ));
        loop_state.observe_managed_command(
            "write_stdin",
            "terminal",
            r#"{"session_id":"command-1","terminal":{"status":"exited","exit_code":1},"output_artifact_ref":"artifact://sha256/output"}"#,
            true,
        );
        assert_eq!(
            ProgressAuditor.audit(
                loop_state.cognitive_state.as_ref().unwrap(),
                &loop_state.evidence_ledger
            ),
            ProgressDecision::Complete
        );
    }
}

fn agent_terminal_evidence(
    capability: &str,
    content: &str,
    is_error: bool,
    spawned_agents: &std::collections::BTreeMap<String, AgentRuntimeId>,
) -> Option<(AgentRuntimeId, TerminalStatus)> {
    if capability != "agent_wait" || is_error {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(content).ok()?;
    if payload.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return None;
    }
    let snapshot = payload.get("result")?;
    let status = match snapshot.get("status").and_then(|value| value.as_str())? {
        "succeeded" => TerminalStatus::Succeeded,
        "failed" => TerminalStatus::Failed,
        "cancelled" | "interrupted" => TerminalStatus::Cancelled,
        _ => return None,
    };
    let runtime = snapshot
        .pointer("/handle/runtime_id")
        .and_then(|value| value.as_str())?;
    let agent_id = snapshot
        .pointer("/handle/agent_id")
        .and_then(|value| value.as_str())?;
    if spawned_agents
        .get(agent_id)
        .map(|runtime| runtime.0.as_str())
        != Some(runtime)
    {
        return None;
    }
    Some((AgentRuntimeId(runtime.to_string()), status))
}

fn agent_spawn_observation(
    capability: &str,
    content: &str,
    is_error: bool,
) -> Option<(String, AgentRuntimeId)> {
    if capability != "agent_spawn" || is_error {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(content).ok()?;
    if payload.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return None;
    }
    let handle = payload.get("result")?;
    let agent_id = handle.get("agent_id")?.as_str()?.to_owned();
    let runtime = handle.get("runtime_id")?.as_str()?.to_owned();
    Some((agent_id, AgentRuntimeId(runtime)))
}

fn streaming_backoff_ms(attempt: u32) -> u64 {
    // Four retries span a typical one-minute provider quota window instead of
    // exhausting every retry in 15 seconds and amplifying a 429 response.
    5_000_u64
        .saturating_mul(1_u64 << attempt.min(3))
        .min(30_000)
}

fn streaming_retry_delay_ms(error: &anyhow::Error, attempt: u32) -> u64 {
    let exponential = streaming_backoff_ms(attempt);
    let provider_advised = error
        .downcast_ref::<crate::adapters::inference::provider::InferenceFailure>()
        .and_then(|failure| failure.retry_after_ms)
        .unwrap_or(0);
    exponential.max(provider_advised)
}

fn exploration_budget_results(
    calls: &[&(String, String, serde_json::Value)],
    content: &str,
    event_sink: &dyn EventSink,
) -> Vec<ContentBlock> {
    calls
        .iter()
        .map(|(id, name, _)| {
            // Canonical history persists tool lifecycle events. Every emitted
            // ToolCallComplete must have a matching ToolResult or the next
            // turn projects an invalid provider tool-call sequence.
            event_sink.emit(Event::ToolResult {
                name: name.clone(),
                call_id: id.clone(),
                result: ToolResultEvent {
                    content: content.to_owned(),
                    is_error: false,
                    execution_time_ms: 0,
                    patch_delta: None,
                    activated_tool_definitions: Vec::new(),
                },
            });
            ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: content.to_owned(),
                is_error: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod streaming_backoff_tests {
    use super::{
        agent_spawn_observation, agent_terminal_evidence, exploration_budget_results,
        streaming_backoff_ms, streaming_retry_delay_ms,
    };
    use crate::core::{AgentRuntimeId, TerminalStatus};
    use crate::harness::event_sink::{Event, EventSink};
    use fabric::ContentBlock;
    use std::sync::Mutex;

    #[derive(Default)]
    struct CollectSink(Mutex<Vec<Event>>);

    impl EventSink for CollectSink {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn retries_span_a_bounded_quota_window() {
        assert_eq!(
            (0..4).map(streaming_backoff_ms).collect::<Vec<_>>(),
            vec![5_000, 10_000, 20_000, 30_000]
        );
    }

    #[test]
    fn streaming_retry_honors_a_longer_provider_retry_after() {
        let error =
            crate::adapters::inference::provider::InferenceFailure::transient_with_retry_after(
                "provider_unavailable",
                Some(42_000),
            );
        assert_eq!(streaming_retry_delay_ms(&error, 0), 42_000);
        assert_eq!(
            streaming_retry_delay_ms(
                &crate::adapters::inference::provider::InferenceFailure::transient(
                    "provider_unavailable"
                ),
                1
            ),
            10_000
        );
    }

    #[test]
    fn budgeted_calls_emit_matching_canonical_results() {
        let calls = [
            (
                "call-1".to_string(),
                "file_read".to_string(),
                serde_json::json!({}),
            ),
            (
                "call-2".to_string(),
                "glob".to_string(),
                serde_json::json!({}),
            ),
        ];
        let refs = calls.iter().collect::<Vec<_>>();
        let sink = CollectSink::default();
        let blocks = exploration_budget_results(&refs, "budget reached", &sink);

        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult { tool_use_id, is_error: false, .. }
            if tool_use_id == "call-1"
        ));
        let events = sink.0.lock().unwrap();
        assert!(matches!(
            &events[0],
            Event::ToolResult { call_id, result, .. }
            if call_id == "call-1" && !result.is_error
        ));
        assert!(matches!(
            &events[1],
            Event::ToolResult { call_id, result, .. }
            if call_id == "call-2" && !result.is_error
        ));
    }

    #[test]
    fn only_terminal_agent_wait_snapshot_creates_runtime_evidence() {
        let mut spawned = std::collections::BTreeMap::new();
        let running = serde_json::json!({
            "ok": true,
            "result": {"handle": {"agent_id": "agent-a", "runtime_id": "runtime-a"}, "status": "running"}
        })
        .to_string();
        assert!(agent_terminal_evidence("agent_wait", &running, false, &spawned).is_none());

        let succeeded = serde_json::json!({
            "ok": true,
            "result": {"handle": {"agent_id": "agent-a", "runtime_id": "runtime-a"}, "status": "succeeded"}
        })
        .to_string();
        assert!(agent_terminal_evidence("agent_wait", &succeeded, false, &spawned).is_none());
        spawned.insert("agent-a".into(), AgentRuntimeId("runtime-a".into()));
        assert_eq!(
            agent_terminal_evidence("agent_wait", &succeeded, false, &spawned),
            Some((
                AgentRuntimeId("runtime-a".into()),
                TerminalStatus::Succeeded
            ))
        );
        assert!(agent_terminal_evidence("agent_spawn", &succeeded, false, &spawned).is_none());

        let spawn = serde_json::json!({
            "ok": true,
            "result": {"agent_id": "agent-a", "runtime_id": "runtime-a"}
        })
        .to_string();
        assert_eq!(
            agent_spawn_observation("agent_spawn", &spawn, false),
            Some(("agent-a".into(), AgentRuntimeId("runtime-a".into())))
        );
    }
}
