use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use aletheon_abi::{Message, ContentBlock, Role, ToolDefinition};
use aletheon_abi::tool::Tool as ToolTrait;
use aletheon_brain::r#impl::learning::{OutcomeRecorder, PatternExtractor, RuleStore};
use aletheon_brain::r#impl::llm::LlmProvider;
use crate::r#impl::memory::compressor::AdvancedCompressor;
use crate::r#impl::memory::core_memory::CoreMemory;
use crate::r#impl::memory::recall_memory::RecallMemory;
use crate::r#impl::orchestration::agent::Agent;
use crate::r#impl::orchestration::delegate::DelegateTool;
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::r#impl::orchestration::selector::SelectorStrategy;
use aletheon_self::bridge::perception::PerceptionInjection;
use aletheon_self::r#impl::hook::dispatcher::HookDispatcher;
use aletheon_self::r#impl::hook::types::{HandlerResult as HookResult, HookContext, HookEventName};
use aletheon_self::r#impl::security::runner::{ToolRunnerWithGuard, ToolError};
use crate::r#impl::session::journal::{EventJournal, SessionEvent};
use aletheon_abi::tool::{ToolContext, ToolResult};
use aletheon_body::r#impl::tools::ToolRegistry;
use aletheon_abi::envelope::Envelope;
use aletheon_comm::CommunicationBus;

use super::config::EngineConfig;

/// Result of a single engine turn with budget awareness.
///
/// Phase 1: wraps the existing `run_turn` return paths.
/// Future phases will add token-budget-aware pause/resume.
#[derive(Debug)]
pub enum TurnResult {
    /// Turn completed normally with assistant response text.
    Complete(String),
    /// A tool call is pending (not yet used in Phase 1; reserved for
    /// future single-step execution mode).
    NeedTool {
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// Reflection pass is needed before continuing (reserved for future use).
    NeedReflection,
    /// Turn ended with an error.
    Error(String),
}

impl TurnResult {
    /// Extract the text if this is a `Complete` result.
    pub fn text(&self) -> Option<&str> {
        match self {
            TurnResult::Complete(s) => Some(s),
            _ => None,
        }
    }

    /// Returns true if the turn finished without errors.
    pub fn is_ok(&self) -> bool {
        matches!(self, TurnResult::Complete(_) | TurnResult::NeedTool { .. } | TurnResult::NeedReflection)
    }
}

/// The ReAct cognitive engine.
pub struct Engine {
    pub(super) llm: Box<dyn LlmProvider>,
    pub(super) tools: ToolRegistry,
    pub(super) config: EngineConfig,
    pub(super) messages: Vec<Message>,
    pub(super) working_dir: std::path::PathBuf,
    pub(super) journal: Option<EventJournal>,
    pub(super) core_memory: Arc<Mutex<CoreMemory>>,
    pub(super) recall_memory: Arc<Mutex<RecallMemory>>,
    pub(super) tool_runner: Option<ToolRunnerWithGuard>,
    pub(super) agent_registry: Option<Arc<AgentRegistry>>,
    pub(super) selector: Option<SelectorStrategy>,
    /// Temporary system prompt override for the current turn.
    pub(super) temp_system_prompt: Option<String>,
    /// Subscriber for perception events from the CommunicationBus topic.
    /// When `None`, falls back to the legacy mpsc receiver.
    pub(super) perception_sub: Option<tokio::sync::broadcast::Receiver<Envelope>>,
    /// Legacy mpsc receiver for backward compatibility when no bus is configured.
    pub(super) perception_rx: Option<tokio::sync::mpsc::Receiver<PerceptionInjection>>,
    /// Hook dispatcher for lifecycle events.
    pub(super) hook_dispatcher: Option<HookDispatcher>,
    /// Advanced context compressor with token-budget tail protection.
    pub(super) compressor: AdvancedCompressor,
    /// CommunicationBus for inter-module communication (request-response, pub-sub).
    /// When `None`, direct lock-based fallback is used for backward compatibility.
    pub(super) bus: Option<Arc<CommunicationBus>>,
    /// Records tool call outcomes for learning (only if learning_enabled).
    pub(super) outcome_recorder: Option<OutcomeRecorder>,
    /// Extracts patterns from historical outcomes.
    pub(super) pattern_extractor: Option<PatternExtractor>,
    /// Stores learned rules and injects them into context.
    pub(super) rule_store: RuleStore,
}

impl Engine {
    pub fn new(
        llm: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        config: EngineConfig,
        working_dir: std::path::PathBuf,
        journal: Option<EventJournal>,
        core_memory: Arc<Mutex<CoreMemory>>,
        recall_memory: Arc<Mutex<RecallMemory>>,
        tool_runner: Option<ToolRunnerWithGuard>,
    ) -> Self {
        let compressor = AdvancedCompressor::new(
            config.tail_token_budget,
            config.target_summary_chars,
        );
        // Initialize learning components if enabled
        let (outcome_recorder, pattern_extractor) = if config.learning_enabled {
            let db_path = working_dir.join(".learning_outcomes.db");
            (
                Some(OutcomeRecorder::new(db_path)),
                Some(PatternExtractor::new(
                    config.learning_min_occurrences,
                    config.learning_success_threshold,
                )),
            )
        } else {
            (None, None)
        };
        let learning_max_rules = config.learning_max_rules;
        let bus = config.bus.clone();

        Self {
            llm,
            tools,
            config,
            messages: Vec::new(),
            working_dir,
            journal,
            core_memory,
            recall_memory,
            tool_runner,
            agent_registry: None,
            selector: None,
            temp_system_prompt: None,
            perception_sub: None,
            perception_rx: None,
            hook_dispatcher: HookDispatcher::try_load(),
            compressor,
            bus,
            outcome_recorder,
            pattern_extractor,
            rule_store: RuleStore::new(learning_max_rules),
        }
    }

    /// Create a new Engine with custom learning components (for testing).
    pub fn with_learning(
        mut self,
        outcome_recorder: OutcomeRecorder,
        pattern_extractor: PatternExtractor,
    ) -> Self {
        self.outcome_recorder = Some(outcome_recorder);
        self.pattern_extractor = Some(pattern_extractor);
        self
    }

    /// Get a reference to the rule store (for testing/inspection).
    pub fn rule_store(&self) -> &RuleStore {
        &self.rule_store
    }

    /// Get a mutable reference to the rule store (for testing).
    pub fn rule_store_mut(&mut self) -> &mut RuleStore {
        &mut self.rule_store
    }

    /// Enable multi-agent orchestration with an existing AgentRegistry.
    ///
    /// After calling this, `delegate_task` tool calls will be routed through
    /// the DelegateTool, and agents can be registered via `register_agent`.
    pub fn with_agent_registry(mut self, registry: Arc<AgentRegistry>) -> Self {
        self.agent_registry = Some(registry);
        self
    }

    /// Set the selector strategy for LLM-based agent routing.
    pub fn with_selector(mut self, selector: SelectorStrategy) -> Self {
        self.selector = Some(selector);
        self
    }

    /// Set the CommunicationBus for inter-module communication.
    pub fn with_bus(mut self, bus: Arc<CommunicationBus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Subscribe to the "perception.events" topic on the CommunicationBus.
    /// Call this when a bus is configured to receive perception events via pub-sub.
    pub fn subscribe_perception(&mut self) {
        if let Some(ref bus) = self.bus {
            let rx = bus.subscribe_topic("perception.events", None);
            self.perception_sub = Some(rx);
        }
    }

    /// Set the perception event receiver from the bridge (legacy mpsc path).
    pub fn set_perception_rx(&mut self, rx: tokio::sync::mpsc::Receiver<PerceptionInjection>) {
        self.perception_rx = Some(rx);
    }

    /// Drain pending perception events and inject into message history.
    ///
    /// Prefers the bus-based broadcast subscriber; falls back to the legacy
    /// mpsc receiver for backward compatibility.
    pub fn drain_perceptions(&mut self) {
        // Try bus-based perception subscription first
        if self.perception_sub.is_some() {
            let mut envelopes = Vec::new();
            let mut closed = false;
            if let Some(rx) = &mut self.perception_sub {
                loop {
                    match rx.try_recv() {
                        Ok(envelope) => {
                            envelopes.push(envelope);
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                            warn!(skipped = n, "Perception subscription lagged, skipping messages");
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                            closed = true;
                            break;
                        }
                    }
                }
            }
            if closed {
                self.perception_sub = None;
            }
            for envelope in &envelopes {
                self.handle_perception_envelope(envelope);
            }
            return;
        }

        // Fallback: legacy mpsc receiver
        if let Some(rx) = &mut self.perception_rx {
            while let Ok(injection) = rx.try_recv() {
                match injection {
                    PerceptionInjection::Immediate(msg) => {
                        // Insert before the last user message
                        let insert_pos = self.messages.iter().rposition(|m| m.role == Role::User)
                            .unwrap_or(self.messages.len());
                        self.messages.insert(insert_pos, msg);
                    }
                    PerceptionInjection::Batch(events) => {
                        if !events.is_empty() {
                            let summary: Vec<String> = events.iter()
                                .map(|e| format!("- {}", e.summary()))
                                .collect();
                            let msg = Message::system(format!(
                                "[Perception Update -- {} events]\n{}",
                                events.len(),
                                summary.join("\n")
                            ));
                            let insert_pos = self.messages.iter().rposition(|m| m.role == Role::User)
                                .unwrap_or(self.messages.len());
                            self.messages.insert(insert_pos, msg);
                        }
                    }
                }
            }
        }
    }

    /// Handle a single perception envelope from the bus topic.
    fn handle_perception_envelope(&mut self, envelope: &Envelope) {
        use super::modules::PerceptionEventMsg;

        if let aletheon_abi::envelope::Payload::Json(val) = &envelope.payload {
            match serde_json::from_value::<PerceptionEventMsg>(val.clone()) {
                Ok(event_msg) => {
                    let msg = Message::system(format!(
                        "[Perception Alert] source={}, priority={}, summary={}",
                        event_msg.source, event_msg.priority, event_msg.summary,
                    ));
                    let insert_pos = self.messages.iter().rposition(|m| m.role == Role::User)
                        .unwrap_or(self.messages.len());
                    self.messages.insert(insert_pos, msg);
                }
                Err(e) => {
                    warn!(error = %e, "Failed to deserialize PerceptionEventMsg from bus envelope");
                }
            }
        }
    }

    /// Register an agent into the orchestration registry.
    ///
    /// Returns an error if no AgentRegistry has been configured
    /// (call `with_agent_registry` first).
    pub async fn register_agent(&self, agent: Arc<RwLock<dyn Agent>>) -> Result<()> {
        let registry = self
            .agent_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("AgentRegistry not configured; call with_agent_registry() first"))?;
        registry.register(agent).await;
        Ok(())
    }

    /// Run the ReAct loop for a single user turn.
    /// Returns the final assistant response text.
    pub async fn run_turn(&mut self, user_input: &str) -> Result<String> {
        // Drain any pending perception events before processing
        self.drain_perceptions();

        // Track if we injected a temp system prompt so we can remove it after
        let injected_temp_system = if let Some(ref sys_prompt) = self.temp_system_prompt {
            self.messages.insert(0, Message::system(sys_prompt));
            true
        } else {
            false
        };

        // Inject core memory into system prompt context (bus-aware)
        let core_memory_content = self.get_core_memory_context().await;
        if !core_memory_content.is_empty() {
            debug!(len = core_memory_content.len(), "Core memory injected into context");
        }

        // Store user message in recall memory (bus-aware)
        self.store_recall(&self.config.session_id, "user", user_input, None).await;

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
        let mut tool_defs = self.get_tool_definitions().await;

        // Add delegate_task tool definition if agent registry is configured
        if self.agent_registry.is_some() {
            let delegate = DelegateTool::new(
                Arc::clone(self.agent_registry.as_ref().unwrap()),
                Default::default(),
            );
            tool_defs.push(ToolDefinition {
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
            debug!(iteration, "ReAct loop iteration");

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

            // Call LLM
            let response = self.llm.complete(&self.messages, &tool_defs).await?;

            // Extract text and tool calls
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();

            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                let final_text = text_parts.join("\n");
                self.messages.push(Message::assistant(&final_text));

                // Store assistant response in recall memory (bus-aware)
                self.store_recall(&self.config.session_id, "assistant", &final_text, None).await;

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
                content: response.content.clone(),
            });

            // Execute tools
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
                        let delegate = DelegateTool::new(
                            Arc::clone(registry),
                            Default::default(),
                        );
                        info!(tool = tool_name.as_str(), "Executing delegate_task via DelegateTool");
                        delegate.execute(tool_input.clone(), &ctx).await
                    } else {
                        // No registry configured -- fall through to normal tool execution
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
                    // Fallback: direct tool execution (no security guards)
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

                // Emit ToolObservationEvent via CommunicationBus (or legacy EventBus fallback)
                if let Some(ref bus) = self.bus {
                    use aletheon_abi::evolution::ToolObservationPayload;
                    use aletheon_comm::core::event::ConcreteEvent;
                    use aletheon_abi::{EventType, Priority};

                    let turn_uuid = uuid::Uuid::parse_str(&turn_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                    let payload = ToolObservationPayload {
                        turn_id: turn_uuid,
                        tool_name: tool_name.to_string(),
                        input: tool_input.clone(),
                        output: serde_json::json!({
                            "content": result.content,
                            "is_error": result.is_error,
                        }),
                        duration_ms: elapsed_ms,
                        error: if result.is_error { Some(result.content.clone()) } else { None },
                        rules_applied: Vec::new(), // Will be populated by BrainCore after reflection
                    };

                    let event = ConcreteEvent::new(
                        EventType::ToolObservation,
                        Priority::Normal,
                        "runtime.engine".to_string(),
                        Box::new(serde_json::to_value(&payload).unwrap_or_default()),
                    );

                    // Fire-and-forget: non-blocking event emission
                    let bus_clone = Arc::clone(bus);
                    tokio::spawn(async move {
                        if let Err(e) = bus_clone.publish_event(Box::new(event)).await {
                            warn!(error = %e, "Failed to publish ToolObservationEvent");
                        }
                    });
                }

                // Add tool result as user message
                self.messages.push(Message::tool_result(
                    tool_id,
                    &result.content,
                    result.is_error,
                ));

                // If turn was interrupted, stop processing remaining tool calls
                if turn_interrupted {
                    break;
                }
            }

            // End loop detector turn tracking if interrupted
            if turn_interrupted {
                if let Some(ref mut runner) = self.tool_runner {
                    runner.end_turn(&turn_id);
                }
                // Clean up temp system prompt
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

    /// Run a single user turn with a token budget.
    ///
    /// Phase 1: delegates to `run_turn` and wraps the result in [`TurnResult`].
    /// The `budget` parameter is accepted but not yet enforced; it will be used
    /// in future phases to pause execution when the energy pool is exhausted.
    ///
    /// # Arguments
    /// * `user_input` - the user's message text
    /// * `budget` - available token budget for this turn (reserved for Phase 2+)
    pub async fn run_turn_with_budget(
        &mut self,
        user_input: &str,
        budget: u32,
    ) -> TurnResult {
        debug!(budget, "run_turn_with_budget called (Phase 1: delegating to run_turn)");
        match self.run_turn(user_input).await {
            Ok(text) => TurnResult::Complete(text),
            Err(e) => TurnResult::Error(e.to_string()),
        }
    }

    /// Get current message history.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Clear message history.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Set a temporary system prompt for the next turn only.
    pub fn set_temp_system_prompt(&mut self, prompt: String) {
        self.temp_system_prompt = Some(prompt);
    }

    /// Clear the temporary system prompt.
    pub fn clear_temp_system_prompt(&mut self) {
        self.temp_system_prompt = None;
    }
}
