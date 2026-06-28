use anyhow::Result;
use tracing::{debug, info, warn};

use aletheon_abi::{Message, ContentBlock, Role};
use aletheon_abi::tool::{ToolResult, Tool as ToolTrait};
use aletheon_brain::r#impl::llm::{StreamChunk, Usage, StopReason};
use futures::StreamExt;

use crate::r#impl::session::journal::SessionEvent;
use aletheon_self::r#impl::hook::types::{HandlerResult as HookResult, HookContext, HookEventName};
use aletheon_self::r#impl::security::runner::ToolError;
use aletheon_abi::tool::ToolContext;

use super::cognitive_loop::Engine;

impl Engine {
    /// Run the ReAct loop for a single user turn with streaming.
    /// Collects streamed chunks into content blocks, then executes tools as needed.
    /// Returns the final assistant response text.
    pub async fn run_turn_streaming(&mut self, user_input: &str) -> Result<String> {
        // Track if we injected a temp system prompt so we can remove it after
        let injected_temp_system = if let Some(ref sys_prompt) = self.temp_system_prompt {
            self.messages.insert(0, Message::system(sys_prompt));
            true
        } else {
            false
        };

        // Inject core memory into system prompt context
        let core_memory_content = {
            let cm = self.core_memory.lock().await;
            cm.format_for_context()
        };
        if !core_memory_content.is_empty() {
            debug!(len = core_memory_content.len(), "Core memory injected into context");
        }

        // Store user message in recall memory
        {
            let rm = self.recall_memory.lock().await;
            if let Err(e) = rm.store(&self.config.session_id, "user", user_input, None) {
                warn!(error = %e, "Failed to store user message in recall memory");
            }
        }

        // Add user message
        self.messages.push(Message::user(user_input));

        // Record user message in journal
        if let Some(j) = &self.journal {
            j.append(SessionEvent::UserMessage {
                content: user_input.to_string(),
            })
            .await?;
        }

        let session_id = self.config.session_id.clone();
        let mut tool_defs = self.tools.definitions();

        // Add delegate_task tool definition if agent registry is configured
        if self.agent_registry.is_some() {
            let delegate = crate::r#impl::orchestration::delegate::DelegateTool::new(
                std::sync::Arc::clone(self.agent_registry.as_ref().unwrap()),
                Default::default(),
            );
            tool_defs.push(aletheon_abi::ToolDefinition {
                name: delegate.name().to_string(),
                description: delegate.description().to_string(),
                input_schema: delegate.input_schema(),
            });
        }

        let turn_id = uuid::Uuid::new_v4().to_string();

        // Notify loop detector of new turn
        if let Some(ref mut runner) = self.tool_runner {
            runner.on_new_turn(&turn_id);
        }

        for iteration in 0..self.config.max_iterations {
            debug!(iteration, "ReAct loop iteration (streaming)");

            // Fire PreLLMCall hooks
            if let Some(ref hd) = self.hook_dispatcher {
                let ctx = HookContext { tool: None, args: None, risk: None, message: None };
                match hd.fire(HookEventName::PreLLMCall, &ctx).await {
                    HookResult::Block(reason) => return Err(anyhow::anyhow!("Blocked by hook: {}", reason)),
                    HookResult::InjectContext(text) => {
                        debug!(len = text.len(), "Hook injected context");
                        self.messages.push(Message::system(text));
                    }
                    _ => {}
                }
            }

            // Inject learned rules into context if learning is enabled
            if self.config.learning_enabled {
                let rules_context = self.rule_store.format_for_context();
                if !rules_context.is_empty() {
                    debug!(len = rules_context.len(), "Injecting learned rules into context");
                    self.messages.push(Message::system(rules_context));
                }
            }

            // Call LLM with streaming
            let mut stream = self.llm.complete_stream(&self.messages, &tool_defs).await?;

            // Collect streamed chunks into content blocks
            let mut content_blocks: Vec<ContentBlock> = Vec::new();
            let mut text_buffer = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut _usage = Usage::default();
            let mut _stop_reason = StopReason::EndTurn;

            // Track tool calls by id for assembly
            let mut tool_inputs: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                match chunk {
                    StreamChunk::TextDelta { text } => {
                        text_buffer.push_str(&text);
                    }
                    StreamChunk::ToolUseStart { id, name } => {
                        tool_inputs.insert(id.clone(), (name, String::new()));
                    }
                    StreamChunk::ToolUseDelta { id, delta } => {
                        if let Some((_, args)) = tool_inputs.get_mut(&id) {
                            args.push_str(&delta);
                        }
                    }
                    StreamChunk::ToolUseComplete { id, input } => {
                        if let Some((name, _)) = tool_inputs.remove(&id) {
                            tool_calls.push((id, name, input));
                        }
                    }
                    StreamChunk::Usage { input_tokens, output_tokens } => {
                        _usage = Usage { input_tokens, output_tokens };
                    }
                    StreamChunk::Done { stop_reason: sr } => {
                        _stop_reason = sr;
                    }
                }
            }

            // Assemble content blocks from collected chunks
            if !text_buffer.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text_buffer.clone() });
            }
            for (id, name, input) in &tool_calls {
                content_blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                let final_text = text_buffer;
                self.messages.push(Message::assistant(&final_text));

                // Store assistant response in recall memory
                {
                    let rm = self.recall_memory.lock().await;
                    if let Err(e) = rm.store(&self.config.session_id, "assistant", &final_text, None) {
                        warn!(error = %e, "Failed to store assistant response in recall memory");
                    }
                }

                // Record assistant message in journal
                if let Some(j) = &self.journal {
                    j.append(SessionEvent::AssistantMessage {
                        content: final_text.clone(),
                    })
                    .await?;
                }

                // Clean up temp system prompt
                if injected_temp_system {
                    if self.messages.first().map(|m| m.role == Role::System).unwrap_or(false) {
                        self.messages.remove(0);
                    }
                    self.temp_system_prompt = None;
                }
                return Ok(final_text);
            }

            // Add assistant message (text + tool_use blocks)
            self.messages.push(Message {
                role: Role::Assistant,
                content: content_blocks,
            });

            // Execute tools (same logic as run_turn)
            let mut turn_interrupted = false;
            for (tool_id, tool_name, tool_input) in &tool_calls {
                let ctx = ToolContext {
                    working_dir: self.working_dir.clone(),
                    session_id: session_id.clone(),
                };

                // Record tool call started
                if let Some(j) = &self.journal {
                    j.append(SessionEvent::ToolCallStarted {
                        tool_call_id: tool_id.clone(),
                        tool_name: tool_name.clone(),
                        input: tool_input.clone(),
                    })
                    .await?;
                }

                let start = std::time::Instant::now();

                // Fire PreToolUse hooks
                if let Some(ref hd) = self.hook_dispatcher {
                    let args_str = serde_json::to_string(tool_input).unwrap_or_default();
                    let ctx = HookContext {
                        tool: Some(tool_name.clone()),
                        args: Some(args_str),
                        risk: None,
                        message: None,
                    };
                    match hd.fire(HookEventName::PreToolUse, &ctx).await {
                        HookResult::Block(reason) => {
                            warn!(tool = tool_name.as_str(), reason = %reason, "Tool blocked by hook");
                            self.messages.push(Message::tool_result(
                                tool_id,
                                &format!("Tool '{}' blocked by hook: {}", tool_name, reason),
                                true,
                            ));
                            continue;
                        }
                        _ => {}
                    }
                }

                // Route delegate_task through DelegateTool when agent_registry is configured
                let result = if tool_name == "delegate_task" {
                    if let Some(ref registry) = self.agent_registry {
                        let delegate = crate::r#impl::orchestration::delegate::DelegateTool::new(
                            std::sync::Arc::clone(registry),
                            Default::default(),
                        );
                        info!(tool = tool_name.as_str(), "Executing delegate_task via DelegateTool");
                        delegate.execute(tool_input.clone(), &ctx).await
                    } else {
                        ToolResult {
                            content: "delegate_task unavailable: no AgentRegistry configured".to_string(),
                            is_error: true,
                            metadata: Default::default(),
                        }
                    }
                } else if let Some(ref mut runner) = self.tool_runner {
                    match self.tools.get(tool_name) {
                        Some(tool) => {
                            info!(tool = tool_name.as_str(), "Executing tool via guarded runner");
                            match runner.execute_tool(
                                tool.as_ref(),
                                tool_input.clone(),
                                &ctx,
                                &turn_id,
                            ).await {
                                Ok(r) => r,
                                Err(ToolError::PolicyDenied { reason }) => {
                                    warn!(tool = tool_name.as_str(), reason = %reason, "Tool denied by policy");
                                    ToolResult {
                                        content: format!("Tool '{}' denied by policy: {}", tool_name, reason),
                                        is_error: true,
                                        metadata: Default::default(),
                                    }
                                }
                                Err(ToolError::LoopBlocked { reason }) => {
                                    warn!(tool = tool_name.as_str(), reason = %reason, "Tool blocked by loop detector");
                                    ToolResult {
                                        content: format!("Tool '{}' blocked (repetitive pattern detected): {}", tool_name, reason),
                                        is_error: true,
                                        metadata: Default::default(),
                                    }
                                }
                                Err(ToolError::EscalateToHuman { reason }) => {
                                    warn!(tool = tool_name.as_str(), reason = %reason, "Tool requires human escalation");
                                    ToolResult {
                                        content: format!("Tool '{}' requires human input: {}", tool_name, reason),
                                        is_error: true,
                                        metadata: Default::default(),
                                    }
                                }
                                Err(ToolError::InterruptTurn { reason }) => {
                                    warn!(tool = tool_name.as_str(), reason = %reason, "Turn interrupted by security guard");
                                    turn_interrupted = true;
                                    ToolResult {
                                        content: format!("Turn interrupted: {}", reason),
                                        is_error: true,
                                        metadata: Default::default(),
                                    }
                                }
                                Err(e) => {
                                    warn!(tool = tool_name.as_str(), error = %e, "Tool execution error");
                                    ToolResult {
                                        content: e.to_string(),
                                        is_error: true,
                                        metadata: Default::default(),
                                    }
                                }
                            }
                        }
                        None => ToolResult {
                            content: format!("Unknown tool: {}", tool_name),
                            is_error: true,
                            metadata: Default::default(),
                        },
                    }
                } else {
                    match self.tools.get(tool_name) {
                        Some(tool) => {
                            info!(tool = tool_name.as_str(), "Executing tool (direct)");
                            tool.execute(tool_input.clone(), &ctx).await
                        }
                        None => ToolResult {
                            content: format!("Unknown tool: {}", tool_name),
                            is_error: true,
                            metadata: Default::default(),
                        },
                    }
                };
                let elapsed_ms = start.elapsed().as_millis() as u64;

                debug!(
                    tool = tool_name.as_str(),
                    is_error = result.is_error,
                    elapsed_ms = elapsed_ms,
                    "Tool result"
                );

                // Record tool call completed
                if let Some(j) = &self.journal {
                    j.append(SessionEvent::ToolCallCompleted {
                        tool_call_id: tool_id.clone(),
                        is_error: result.is_error,
                        content: result.content.clone(),
                        elapsed_ms,
                    })
                    .await?;
                }

                // Record learning outcome if enabled
                if self.config.learning_enabled {
                    self.record_tool_outcome(
                        &session_id,
                        &turn_id,
                        tool_name,
                        tool_input,
                        &result,
                        iteration,
                    ).await;
                }

                // Add tool result as user message
                self.messages.push(Message::tool_result(
                    tool_id,
                    &result.content,
                    result.is_error,
                ));

                if turn_interrupted {
                    break;
                }
            }

            // End loop detector turn tracking if interrupted
            if turn_interrupted {
                if let Some(ref mut runner) = self.tool_runner {
                    runner.end_turn(&turn_id);
                }
                if injected_temp_system {
                    if self.messages.first().map(|m| m.role == Role::System).unwrap_or(false) {
                        self.messages.remove(0);
                    }
                    self.temp_system_prompt = None;
                }
                return Err(anyhow::anyhow!("Turn interrupted by security guard"));
            }

            // Advanced context compaction with token-budget tail protection
            if self.config.compaction_enabled {
                let old_count = self.messages.len();
                if self.compressor.maybe_compact(
                    &mut self.messages,
                    &*self.llm,
                ).await? {
                    if let Some(j) = &self.journal {
                        j.append(SessionEvent::Compacted {
                            before_count: old_count,
                            after_count: self.messages.len(),
                        })
                        .await?;
                    }
                }
            }

            // Record checkpoint boundary
            if let Some(j) = &self.journal {
                j.append(SessionEvent::CheckpointBoundary { iteration })
                    .await?;
            }
        }

        // Clean up temp system prompt
        if injected_temp_system {
            if self.messages.first().map(|m| m.role == Role::System).unwrap_or(false) {
                self.messages.remove(0);
            }
            self.temp_system_prompt = None;
        }

        Err(anyhow::anyhow!(
            "Max iterations ({}) exceeded",
            self.config.max_iterations
        ))
    }
}
