mod step;
mod tool_exec;
pub mod tool_budget;
pub mod circuit_breaker;
pub mod goal_tracker;
pub mod reflection;

use tool_budget::ToolBudget;
use circuit_breaker::{CircuitBreaker, CircuitBreakerStatus, ToolCallSignature};
use goal_tracker::GoalTracker;
use reflection::ReflectionEngine;

use crate::core::config::RuntimeConfig;
use crate::core::interrupt::InterruptFlag;
use crate::r#impl::memory::compressor::AdvancedCompressor;
use base::tool::ConcurrencyClass;
use base::ui_event::AwarenessLevel;
use base::policy::verifier::{Verdict, Verifier};
use std::sync::Arc;
use cognit::core::awareness_signal::{self, AwarenessSignal, StepType};

/// Marker injected into user messages when plan mode is active.
/// Shared between `ReActLoop` and `Controller` to keep them in sync.
pub const PLAN_MODE_MARKER: &str = "[PLAN MODE ACTIVE]";
use base::body::Action;
use base::message::{ContentBlock, Message, Role};
use base::self_field::{Intent, IntentSource};
use base::ToolDefinition;

/// Metrics collected during a single ReAct turn.
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
}

/// Maximum number of tools to execute in a single parallel batch.
const MAX_PARALLEL_TOOLS: usize = 8;

/// A batch of tool calls classified for parallel or serial execution.
pub enum ToolBatch {
    /// Read-only tools safe to execute concurrently.
    Parallel(Vec<(String, String, serde_json::Value)>),
    /// Side-effect tools that must be executed sequentially.
    Serial(Vec<(String, String, serde_json::Value)>),
}

/// Classify a tool by name into its concurrency class.
fn classify_tool(tool_name: &str) -> ConcurrencyClass {
    match tool_name {
        "read_file" | "glob" | "grep" | "file_read" | "system_status"
        | "process_list" | "memory_search" | "ls" | "web_fetch" | "web_search" => {
            ConcurrencyClass::ReadOnly
        }
        _ => ConcurrencyClass::SideEffect,
    }
}

/// Partition a list of tool calls into parallel and serial batches.
///
/// Contiguous read-only tools are grouped into a single `Parallel` batch
/// (up to `MAX_PARALLEL_TOOLS`). Each side-effect tool gets its own `Serial`
/// batch. A switch from read-only to side-effect (or vice versa) flushes the
/// current batch.
pub fn partition_tool_calls(
    calls: &[(String, String, serde_json::Value)],
) -> Vec<ToolBatch> {
    if calls.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut current_readonly: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut current_serial: Vec<(String, String, serde_json::Value)> = Vec::new();

    let flush_readonly = |batches: &mut Vec<ToolBatch>,
                          buf: &mut Vec<(String, String, serde_json::Value)>| {
        if !buf.is_empty() {
            batches.push(ToolBatch::Parallel(std::mem::take(buf)));
        }
    };

    let flush_serial = |batches: &mut Vec<ToolBatch>,
                        buf: &mut Vec<(String, String, serde_json::Value)>| {
        if !buf.is_empty() {
            batches.push(ToolBatch::Serial(std::mem::take(buf)));
        }
    };

    for call in calls {
        let class = classify_tool(&call.1);
        match class {
            ConcurrencyClass::ReadOnly => {
                flush_serial(&mut batches, &mut current_serial);
                current_readonly.push(call.clone());
                if current_readonly.len() >= MAX_PARALLEL_TOOLS {
                    flush_readonly(&mut batches, &mut current_readonly);
                }
            }
            _ => {
                flush_readonly(&mut batches, &mut current_readonly);
                current_serial.push(call.clone());
                // Each side-effect tool gets its own serial batch.
                flush_serial(&mut batches, &mut current_serial);
            }
        }
    }

    flush_readonly(&mut batches, &mut current_readonly);
    flush_serial(&mut batches, &mut current_serial);

    batches
}

use cognit::r#impl::llm::provider::{LlmProvider, StopReason, StreamChunk};
use std::future::Future;
use tracing::{debug, warn};

use crate::core::event_sink::{Event, EventSink, ToolResultEvent};

/// The ReAct (Reason + Act) iteration loop
/// This is the core cognitive cycle extracted from Engine::run_turn()
pub struct ReActLoop {
    config: RuntimeConfig,
    iteration: usize,
    messages: Vec<Message>,
    compressor: AdvancedCompressor,
    /// Immutable system prompt — never changes after construction.
    system_prompt: String,
    /// Plan mode flag — injected into user message, NOT system prompt.
    plan_mode: bool,
    /// Pending memory updates — drained into user message each turn.
    pending_memory: Vec<String>,
    /// Collected awareness signals during the current turn.
    signals: Vec<AwarenessSignal>,
    /// Recent tool names for goal-shift detection.
    recent_tools: Vec<String>,
    /// Consecutive tool errors for impasse detection.
    consecutive_errors: usize,
    /// Interrupt flag for canceling the loop externally.
    interrupt_flag: Option<InterruptFlag>,
    /// Tool call budget per turn.
    tool_budget: ToolBudget,
    /// Circuit breaker for loop detection.
    circuit_breaker: CircuitBreaker,
    /// Goal and sub-goal tracker.
    goal_tracker: GoalTracker,
    /// Periodic reflection engine.
    reflection_engine: ReflectionEngine,
    /// Optional result verifier (M-C). None = no-op (unchanged behavior).
    verifier: Option<Arc<dyn Verifier>>,
    /// Verify attempts used this turn (reset at the start of run()).
    verify_attempts: usize,
    /// Max verify-reject retries per turn before returning as-is.
    max_verify_attempts: usize,
}

impl ReActLoop {
    pub fn new(config: RuntimeConfig) -> Self {
        // Scale tail_token_budget proportionally to context_window_tokens.
        // Default config has tail_token_budget=16K for 128K context (~12.5%).
        // For larger contexts, scale up so compaction doesn't fire too early.
        let effective_tail = if config.tail_token_budget * 4 < config.context_window_tokens {
            // tail_token_budget is less than 25% of context window — scale up
            config.context_window_tokens / 8  // ~12.5% of context
        } else {
            config.tail_token_budget
        };
        let compressor =
            AdvancedCompressor::new(effective_tail, config.target_summary_chars, config.context_window_tokens);
        let tool_budget = ToolBudget::new(config.agent_loop.max_tool_calls);
        let circuit_breaker = CircuitBreaker::new(
            config.circuit_breaker.max_repeats,
            config.circuit_breaker.window_size,
        );
        let goal_tracker = GoalTracker::new();
        let reflection_engine = ReflectionEngine::new(config.agent_loop.reflection_interval);

        Self {
            config,
            iteration: 0,
            messages: Vec::new(),
            compressor,
            system_prompt: String::new(),
            plan_mode: false,
            pending_memory: Vec::new(),
            signals: Vec::new(),
            recent_tools: Vec::new(),
            consecutive_errors: 0,
            interrupt_flag: None,
            tool_budget,
            circuit_breaker,
            goal_tracker,
            reflection_engine,
            verifier: None,
            verify_attempts: 0,
            max_verify_attempts: 2,
        }
    }

    /// Number of messages in the conversation buffer.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Current iteration number
    pub fn iteration(&self) -> usize {
        self.iteration
    }

    /// Reset iteration counter for a new turn.
    /// Clears mutable state (messages, pending_memory) but preserves
    /// plan_mode and system_prompt (user choice / immutable).
    pub fn reset(&mut self) {
        self.iteration = 0;
        self.messages.clear();
        self.pending_memory.clear();
        self.signals.clear();
        self.recent_tools.clear();
        self.consecutive_errors = 0;
        self.tool_budget.reset();
        self.circuit_breaker.reset();
        self.goal_tracker.reset();
        self.reflection_engine.reset();
        // Note: plan_mode persists across resets (user choice)
        // Note: system_prompt never resets (immutable after construction)
    }

    /// Seed the message buffer with pre-existing messages (e.g., from session restore).
    pub fn seed_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Seed the goal tracker from persisted state (resume-on-start).
    /// Must be called before the first turn; subsequent turns' reset() is unaffected.
    pub fn seed_goal(&mut self, description: &str, sub_goals: &[String]) {
        self.goal_tracker.hydrate_from(description, sub_goals);
    }

    /// Check if we've hit the max iterations
    pub fn should_continue(&self) -> bool {
        self.iteration < self.config.max_iterations
    }

    /// Increment iteration counter
    pub fn advance(&mut self) {
        self.iteration += 1;
    }

    /// Build an Intent from user input
    pub fn build_intent(&self, input: &str) -> Intent {
        Intent {
            action: "user_request".to_string(),
            parameters: serde_json::json!({"input": input}),
            source: IntentSource::User,
            description: input.to_string(),
        }
    }

    /// Build an Action from a plan step
    pub fn step_to_action(&self, tool_name: &str, params: serde_json::Value) -> Action {
        Action {
            name: tool_name.to_string(),
            parameters: params,
            requires_sandbox: false,
            timeout: None,
        }
    }

    /// Max iterations
    pub fn max_iterations(&self) -> usize {
        self.config.max_iterations
    }

    /// Set the system prompt (called once at construction or re-initialization).
    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    /// Get the immutable system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Set the interrupt flag for external cancellation.
    pub fn set_interrupt_flag(&mut self, flag: InterruptFlag) {
        self.interrupt_flag = Some(flag);
    }

    /// Install a result verifier. Without this, verification is a no-op.
    pub fn set_verifier(&mut self, verifier: Arc<dyn Verifier>) {
        self.verifier = Some(verifier);
    }

    /// Set the goal for this turn.
    pub fn set_goal(&mut self, goal: String) {
        self.goal_tracker.set_goal(goal);
    }

    /// Load a spec file into the goal tracker.
    pub fn load_spec(&mut self, path: &str) -> Result<(), String> {
        self.goal_tracker.load_spec_from_file(path)
    }

    /// Get the current constraints from the loaded spec.
    pub fn get_constraints(&self) -> &[String] {
        self.goal_tracker.get_constraints()
    }

    /// Get the current goal context for LLM prompting.
    pub fn get_goal_context(&self) -> String {
        self.goal_tracker.get_context()
    }

    /// Enable/disable plan mode. Injected into user message, NOT system prompt.
    pub fn set_plan_mode(&mut self, enabled: bool) {
        self.plan_mode = enabled;
    }

    /// Queue a memory update for the next user message.
    pub fn queue_memory_update(&mut self, update: String) {
        self.pending_memory.push(update);
    }

    /// Compose user message with mid-session injections.
    /// Changes go here, NOT into system prompt, to preserve cache stability.
    pub fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        if self.plan_mode {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        if !self.pending_memory.is_empty() {
            let updates = self
                .pending_memory
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        let goal_ctx = self.goal_tracker.get_context();
        if !goal_ctx.is_empty() {
            parts.push(format!("<goal-context>\n{}\n</goal-context>", goal_ctx));
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }

    /// Compose user message with mid-session injections plus DaseinContext.
    ///
    /// The DaseinContext is injected as a `<dasein-state>` XML block in the
    /// user message (not the system prompt) to preserve cache stability.
    /// This lets the LLM perceive the system's existential state --
    /// mood, temporal flow, involvement network, and care structure.
    pub fn compose_user_message_with_dasein(
        &self,
        input: &str,
        dasein_context: Option<&str>,
    ) -> String {
        let mut parts = Vec::new();

        if self.plan_mode {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        if !self.pending_memory.is_empty() {
            let updates = self
                .pending_memory
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        let goal_ctx = self.goal_tracker.get_context();
        if !goal_ctx.is_empty() {
            parts.push(format!("<goal-context>\n{}\n</goal-context>", goal_ctx));
        }

        if let Some(ctx) = dasein_context {
            if !ctx.is_empty() {
                parts.push(format!("<dasein-state>\n{}\n</dasein-state>", ctx));
            }
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }

    // --- Awareness signal helpers ---

    /// Emit an awareness signal into the collection buffer.
    fn emit_signal(&mut self, signal: AwarenessSignal) {
        self.signals.push(signal);
    }

    /// Drain collected awareness signals, leaving the buffer empty.
    pub fn take_signals(&mut self) -> Vec<AwarenessSignal> {
        std::mem::take(&mut self.signals)
    }

    /// Drain accumulated awareness signals as UI events.
    ///
    /// Returns `(AwarenessLevel, context)` pairs suitable for TUI display.
    /// Signals with no detected state or unrecognized states are filtered out.
    pub fn drain_awareness_events(&mut self) -> Vec<(AwarenessLevel, String)> {
        let signals: Vec<_> = self.signals.drain(..).collect();
        awareness_signal::signals_to_ui_events(&signals)
    }

    /// Emit a LoopStart signal with impasse detection.
    fn emit_loop_start(&mut self, action: &str) {
        use cognit::core::awareness_signal::detect_impasse;
        let detected = detect_impasse(
            self.consecutive_errors,
            self.iteration,
            self.config.max_iterations,
        );
        self.emit_signal(AwarenessSignal {
            step: StepType::LoopStart,
            action: action.to_string(),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a ThinkingComplete signal with uncertainty detection from response text.
    fn emit_thinking_complete(&mut self, action: &str, response_text: &str) {
        use cognit::core::awareness_signal::detect_uncertainty;
        let detected = detect_uncertainty(response_text);
        self.emit_signal(AwarenessSignal {
            step: StepType::ThinkingComplete,
            action: action.to_string(),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a ToolCallEnd signal with impasse detection from consecutive errors.
    fn emit_tool_call_end(&mut self, tool_name: &str) {
        use cognit::core::awareness_signal::{detect_goal_shift, detect_impasse};

        // Track tool name for goal-shift detection
        self.recent_tools.push(tool_name.to_string());

        let mut detected = None;

        // Check impasse from errors
        if let Some(state) = detect_impasse(
            self.consecutive_errors,
            self.iteration,
            self.config.max_iterations,
        ) {
            detected = Some(state);
        }

        // Check goal shift from tool sequence
        if detected.is_none() {
            detected = detect_goal_shift(&self.recent_tools);
        }

        self.emit_signal(AwarenessSignal {
            step: StepType::ToolCallEnd,
            action: format!("tool:{}", tool_name),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a FinalResponse signal with Focused state.
    fn emit_final_response(&mut self, action: &str) {
        self.emit_signal(AwarenessSignal {
            step: StepType::FinalResponse,
            action: action.to_string(),
            detected_state: Some(base::self_field::SelfState::Focused),
            timestamp: chrono::Utc::now(),
        });
    }
}

/// Check if an error indicates a context window overflow.
fn is_context_overflow(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("context")
        || msg.contains("too long")
        || msg.contains("maximum context")
        || msg.contains("prompt is too long")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::message::{ContentBlock, Message};
    use base::ToolDefinition;
    use cognit::r#impl::llm::provider::{
        LlmProvider, LlmResponse, LlmStream, StopReason, Usage,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct ScriptedLlm {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "echo_tool".into(),
                        input: serde_json::json!({"text": "hi"}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "done: hi".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "scripted"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn interleaved_loop_executes_tool_then_finishes() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: false,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = ScriptedLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let executed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executed2 = executed.clone();

        let (out, metrics) = lp
            .run(
                "make hi",
                &llm,
                &tool_defs,
                |_id: &str, name: &str, _input: &serde_json::Value| {
                    let executed = executed2.clone();
                    let name = name.to_string();
                    async move {
                        executed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        (format!("ran {}", name), false)
                    }
                },
            )
            .await
            .unwrap();

        assert_eq!(
            executed.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "tool ran exactly once"
        );
        assert!(out.contains("done"), "final text returned: {out}");
        assert_eq!(metrics.tool_calls_made, 1);
        assert_eq!(metrics.tool_errors, 0);
        assert!(metrics.completed_normally);
    }

    /// An LLM that always returns tool-use, then ends with text on the Nth call.
    struct BigToolLlm {
        calls: Mutex<usize>,
        tool_until: usize,
    }

    #[async_trait]
    impl LlmProvider for BigToolLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n <= self.tool_until {
                let big_text = "x".repeat(10_000);
                Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: format!("call_{}", n),
                        name: "big_tool".into(),
                        input: serde_json::json!({"data": big_text}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "big_tool"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn loop_compacts_when_over_budget() {
        // tail_token_budget=5000 so the tail can hold ~3 messages (~7500 tokens).
        // Messages from tool interactions are ~2500 tokens each, so compaction
        // triggers when total > 80% of context_window_tokens.
        // Set context_window_tokens=12_500 so 80% threshold = 10_000 tokens (about 4 messages).
        let cfg = RuntimeConfig {
            max_iterations: 30,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 5_000,
            target_summary_chars: 200,
            context_window_tokens: 12_500,
            agent_loop: crate::core::config::AgentLoopConfig {
                max_tool_calls: 25, // Higher threshold for this compaction test
                reflection_interval: 30, // Disable reflection for this compaction test
                ..Default::default()
            },
            circuit_breaker: crate::core::config::CircuitBreakerConfig {
                max_repeats: 25, // Higher threshold for this compaction test
                window_size: 50,
            },
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = BigToolLlm {
            calls: Mutex::new(0),
            tool_until: 20,
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let (out, _metrics) = lp
            .run(
                "do many big things",
                &llm,
                &tool_defs,
                |_id: &str, name: &str, _input: &serde_json::Value| {
                    let name = name.to_string();
                    let big = "y".repeat(10_000);
                    async move { (format!("result_{}: {}", name, big), false) }
                },
            )
            .await
            .unwrap();

        assert!(out.contains("done"), "final text returned: {out}");
        // Without compaction we'd have 1 + 20*2 + 1 = 42 messages.
        // With compaction, the count should be significantly bounded.
        let count = lp.message_count();
        assert!(
            count < 20,
            "Expected message count bounded by compaction, got {count}"
        );
    }

    /// An LLM that errors "prompt is too long" on the first call, then succeeds.
    struct ErrorThenOkLlm {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for ErrorThenOkLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Err(anyhow::anyhow!("prompt is too long: 200000 tokens"))
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "recovered after compaction".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "error_then_ok"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn reactive_compaction_on_context_overflow() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 1_000,
            target_summary_chars: 100,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        // Seed with some messages to give compaction something to work with
        lp.messages = vec![
            Message::user("old message 1"),
            Message::assistant("old response 1"),
            Message::user("old message 2"),
            Message::assistant("old response 2"),
        ];
        let llm = ErrorThenOkLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let (out, _metrics) = lp
            .run(
                "trigger overflow",
                &llm,
                &tool_defs,
                |_id: &str, _name: &str, _input: &serde_json::Value| {
                    async move { ("tool result".into(), false) }
                },
            )
            .await
            .unwrap();

        assert!(
            out.contains("recovered"),
            "Should recover after compaction: {out}"
        );
    }

    /// An LLM that returns no text and tool calls on first iteration,
    /// then returns text on second.
    struct EmptyThenTextLlm {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for EmptyThenTextLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Ok(LlmResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text {
                        text: "finally text".into(),
                    }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 0,
                })
            }
        }

        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }

        fn name(&self) -> &str {
            "empty_then_text"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn empty_content_still_returns_text() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: false,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        let llm = EmptyThenTextLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let (out, metrics) = lp
            .run(
                "test",
                &llm,
                &tool_defs,
                |_id: &str, _name: &str, _input: &serde_json::Value| {
                    async move { ("result".into(), false) }
                },
            )
            .await
            .unwrap();

        // Empty content blocks -> still returns a string (possibly empty)
        // The loop should complete normally
        assert!(metrics.completed_normally);
        // On first call, content is empty, so text_parts.join returns ""
        // On second call (since no tool calls), it returns "finally text"
        assert!(out.contains("finally text") || out.is_empty());
    }

    // ── Compose message tests ────────────────────────────────────────────────

    #[test]
    fn compose_user_message_plain_input() {
        let cfg = RuntimeConfig::default();
        let lp = ReActLoop::new(cfg);
        assert_eq!(lp.compose_user_message("hello"), "hello");
    }

    #[test]
    fn compose_user_message_with_plan_mode() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        let msg = lp.compose_user_message("hello");
        assert!(msg.contains(PLAN_MODE_MARKER));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_memory_updates() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.queue_memory_update("user prefers dark mode".into());
        let msg = lp.compose_user_message("hello");
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("user prefers dark mode"));
    }

    #[test]
    fn compose_user_message_plan_and_memory() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        lp.queue_memory_update("fact".into());
        let msg = lp.compose_user_message("hi");
        assert!(msg.contains(PLAN_MODE_MARKER));
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("hi"));
    }

    #[test]
    fn system_prompt_immutable_after_construction() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        assert_eq!(lp.system_prompt(), "");
        lp.set_system_prompt("test prompt".into());
        assert_eq!(lp.system_prompt(), "test prompt");
    }

    #[test]
    fn reset_clears_pending_memory_but_not_plan_mode() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        lp.queue_memory_update("test".into());
        lp.reset();
        // plan_mode persists
        assert!(lp.plan_mode);
        // pending_memory cleared
        assert!(lp.pending_memory.is_empty());
    }

    #[test]
    fn set_system_prompt_works() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_system_prompt("You are helpful".into());
        assert_eq!(lp.system_prompt(), "You are helpful");
    }

    #[test]
    fn compose_multiple_memory_updates() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.queue_memory_update("fact 1".into());
        lp.queue_memory_update("fact 2".into());
        let msg = lp.compose_user_message("hi");
        assert!(msg.contains("fact 1"));
        assert!(msg.contains("fact 2"));
    }

    #[test]
    fn compose_user_message_with_dasein_injection() {
        let cfg = RuntimeConfig::default();
        let lp = ReActLoop::new(cfg);
        let msg = lp.compose_user_message_with_dasein("hello", Some("mood: curious"));
        assert!(msg.contains("<dasein-state>"));
        assert!(msg.contains("mood: curious"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_dasein_none() {
        let cfg = RuntimeConfig::default();
        let lp = ReActLoop::new(cfg);
        let msg = lp.compose_user_message_with_dasein("hello", None);
        assert!(!msg.contains("<dasein-state>"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_dasein_empty() {
        let cfg = RuntimeConfig::default();
        let lp = ReActLoop::new(cfg);
        let msg = lp.compose_user_message_with_dasein("hello", Some(""));
        assert!(!msg.contains("<dasein-state>"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_all_injections() {
        let cfg = RuntimeConfig::default();
        let mut lp = ReActLoop::new(cfg);
        lp.set_plan_mode(true);
        lp.queue_memory_update("remember this".into());
        let msg = lp.compose_user_message_with_dasein("do task", Some("temporal: present"));
        assert!(msg.contains(PLAN_MODE_MARKER));
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("<dasein-state>"));
        assert!(msg.contains("do task"));
    }

    // ── Partition tests ──────────────────────────────────────────────────────

    #[test]
    fn partition_read_only_batch() {
        let calls = vec![
            ("1".into(), "read_file".into(), serde_json::json!({})),
            ("2".into(), "glob".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        assert_eq!(batches.len(), 1);
        assert!(matches!(batches[0], ToolBatch::Parallel(_)));
    }

    #[test]
    fn partition_writer_serial() {
        let calls = vec![
            ("1".into(), "write_file".into(), serde_json::json!({})),
            ("2".into(), "bash_exec".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        // Each side-effect tool gets its own serial batch
        assert_eq!(batches.len(), 2);
        for b in &batches {
            assert!(matches!(b, ToolBatch::Serial(_)));
        }
    }

    #[test]
    fn partition_mixed() {
        let calls = vec![
            ("1".into(), "read_file".into(), serde_json::json!({})),
            ("2".into(), "write_file".into(), serde_json::json!({})),
            ("3".into(), "grep".into(), serde_json::json!({})),
        ];
        let batches = partition_tool_calls(&calls);
        assert_eq!(batches.len(), 3);
        assert!(matches!(batches[0], ToolBatch::Parallel(_)));
        assert!(matches!(batches[1], ToolBatch::Serial(_)));
        assert!(matches!(batches[2], ToolBatch::Parallel(_)));
    }

    #[test]
    fn partition_empty() {
        let batches = partition_tool_calls(&[]);
        assert!(batches.is_empty());
    }

    // ── M-C Verifier tests ─────────────────────────────────────────────────

    use base::policy::verifier::{Verdict, Verifier};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Rejects the first candidate answer, accepts all subsequent ones.
    struct RejectOnce {
        seen: AtomicUsize,
    }
    #[async_trait]
    impl Verifier for RejectOnce {
        async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
            if self.seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Verdict::Reject { reason: "first try rejected".into() }
            } else {
                Verdict::Accept
            }
        }
    }

    /// An LLM that always returns plain text (no tool calls), counting its calls.
    struct TextLlm {
        calls: Mutex<usize>,
    }
    #[async_trait]
    impl LlmProvider for TextLlm {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            Ok(LlmResponse {
                content: vec![ContentBlock::Text { text: format!("answer {n}") }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }
        fn name(&self) -> &str { "text" }
        fn max_context_length(&self) -> usize { 100_000 }
    }

    #[tokio::test]
    async fn verifier_rejection_triggers_one_retry() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: false,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        lp.set_verifier(std::sync::Arc::new(RejectOnce { seen: AtomicUsize::new(0) }));
        let llm = TextLlm { calls: Mutex::new(0) };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp
            .run("go", &llm, &tool_defs, |_id: &str, name: &str, _in: &serde_json::Value| {
                let name = name.to_string();
                async move { (format!("ran {name}"), false) }
            })
            .await
            .unwrap();
        // First answer rejected -> loop retried -> second answer accepted.
        assert_eq!(out, "answer 2", "rejected answer should be revised, got: {out}");
    }

    #[tokio::test]
    async fn no_verifier_returns_first_answer_unchanged() {
        let cfg = RuntimeConfig { max_iterations: 5, session_id: "t".into(),
            learning_enabled: false, compaction_enabled: false, ..RuntimeConfig::default() };
        let mut lp = ReActLoop::new(cfg); // no set_verifier -> None
        let llm = TextLlm { calls: Mutex::new(0) };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp.run("go", &llm, &tool_defs,
            |_i: &str, n: &str, _in: &serde_json::Value| { let n = n.to_string(); async move { (n, false) } })
            .await.unwrap();
        assert_eq!(out, "answer 1", "no verifier = unchanged behavior");
    }
}
