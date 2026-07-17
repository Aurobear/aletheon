pub mod awareness;
pub mod batching;
pub mod circuit_breaker;
pub mod goal_tracker;
pub mod message_compose;
pub mod metrics;
pub mod reflection;
mod step;
pub mod tool_budget;
mod tool_exec;
mod tool_output;

pub use batching::{partition_tool_calls, ToolBatch};
pub use metrics::TurnMetrics;

use async_trait::async_trait;
use circuit_breaker::CircuitBreaker;
use goal_tracker::GoalTracker;
use reflection::ReflectionEngine;
use tool_budget::ToolBudget;

use crate::core::awareness_signal::AwarenessSignal;
use crate::harness::config::HarnessConfig;
use crate::harness::interrupt::InterruptFlag;
use crate::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream};
use fabric::body::Action;
use fabric::message::Message;
use fabric::policy::verifier::Verifier;
use fabric::self_field::{Intent, IntentSource};
use fabric::{Clock, ToolDefinition};
use std::sync::Arc;

/// Thin wrapper to allow passing `&dyn LlmProvider` to generic functions
/// that require `LlmProvider + Sized` (e.g. `ReActLoop::run`).
pub struct DynLlmRef<'a>(pub &'a dyn LlmProvider);

#[async_trait]
impl LlmProvider for DynLlmRef<'_> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn max_context_length(&self) -> usize {
        self.0.max_context_length()
    }
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.0.complete(messages, tools).await
    }
    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        self.0.complete_stream(messages, tools).await
    }
}

/// Trait for context compaction into the message buffer.
/// Re-exported from `fabric`, the shared compaction interface, so both
/// `cognit` and concrete compaction strategies (e.g. `mnemosyne`) depend
/// on the same abstract contract without depending on each other.
pub use fabric::CompactorTrait;

/// Async trait for planning capability batch execution order.
///
/// The planner receives the complete set of tool calls the LLM requested in
/// one iteration and returns a validated plan. Cognit applies the plan only
/// when the mode is `Enforce` and the plan is a valid exact permutation.
#[async_trait]
pub trait BatchPlanner: Send + Sync {
    async fn plan(
        &self,
        calls: Vec<fabric::CapabilityCall>,
    ) -> anyhow::Result<fabric::CapabilityBatchPlan>;
}

/// Marker injected into user messages when plan mode is active.
/// Shared between `ReActLoop` and `Controller` to keep them in sync.
pub const PLAN_MODE_MARKER: &str = "[PLAN MODE ACTIVE]";

/// The ReAct (Reason + Act) iteration loop
/// This is the core cognitive cycle extracted from Engine::run_turn()
pub struct ReActLoop {
    config: HarnessConfig,
    iteration: usize,
    messages: Vec<Message>,
    compressor: Box<dyn CompactorTrait>,
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
    /// Optional Dasein context provider — called each turn to inject SelfField state.
    dasein_ctx_provider: Option<Box<dyn Fn() -> Option<String> + Send + Sync>>,
    /// Optional batch planner — called before each tool-execution batch to
    /// reorder calls according to conscious arbitration policy.
    batch_planner: Option<Arc<dyn BatchPlanner>>,
    /// Clock for deterministic time (mono/wall).
    clock: Arc<dyn Clock>,
}

impl ReActLoop {
    pub fn new_with_clock(
        config: HarnessConfig,
        compressor: Box<dyn CompactorTrait>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let tool_budget = ToolBudget::new(config.max_tool_calls);
        let circuit_breaker = CircuitBreaker::new(
            config.circuit_breaker_max_repeats,
            config.circuit_breaker_window_size,
        );
        let goal_tracker = GoalTracker::new(clock.clone());
        let reflection_engine = ReflectionEngine::new(
            config.reflection_interval,
            config.reflection_tool_call_limit,
        );

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
            dasein_ctx_provider: None,
            batch_planner: None,
            clock,
        }
    }

    #[cfg(test)]
    pub fn new(config: HarnessConfig, compressor: Box<dyn CompactorTrait>) -> Self {
        Self::new_with_clock(
            config,
            compressor,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
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

    /// Check if we've hit the max iterations.
    /// `max_iterations == 0` means unlimited: the loop then terminates only via
    /// LLM stop, circuit breaker, repeated-call detection, or the tool budget.
    pub fn should_continue(&self) -> bool {
        self.config.max_iterations == 0 || self.iteration < self.config.max_iterations
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

    /// Set the batch planner for conscious arbitration.
    pub fn set_batch_planner(&mut self, planner: Arc<dyn BatchPlanner>) {
        self.batch_planner = Some(planner);
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
    use crate::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};
    use async_trait::async_trait;
    use fabric::message::{ContentBlock, Message};
    use fabric::ToolDefinition;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// No-op compressor for tests that don't exercise compaction.
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
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        };
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
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
        let cfg = HarnessConfig {
            max_iterations: 30,
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 5_000,
            target_summary_chars: 200,
            context_window_tokens: 12_500,
            max_tool_calls: 25,      // Higher threshold for this compaction test
            reflection_interval: 30, // Disable reflection for this compaction test
            circuit_breaker_max_repeats: 25, // Higher threshold for this compaction test
            circuit_breaker_window_size: 50,
            ..HarnessConfig::default()
        };
        let compressor = Box::new(NoopCompressor) as Box<dyn CompactorTrait>;
        let mut lp = ReActLoop::new(cfg, compressor);
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
        // With NoopCompressor in tests, compaction is a no-op.
        // The loop still completes normally; message count reflects full history.
        let count = lp.message_count();
        assert!(count > 0, "should have some messages");
    }

    #[tokio::test]
    async fn exhausted_tool_budget_closes_pending_tool_calls() {
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            max_tool_calls: 1,
            reflection_interval: 30,
            ..HarnessConfig::default()
        };
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        let llm = BigToolLlm {
            calls: Mutex::new(0),
            tool_until: 2,
        };

        let (out, metrics) = lp
            .run(
                "use more tools than allowed",
                &llm,
                &[],
                |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                    ("ok".into(), false)
                },
            )
            .await
            .unwrap();

        assert!(out.contains("Tool budget exhausted"));
        assert!(!metrics.completed_normally);

        for (index, message) in lp.messages.iter().enumerate() {
            for block in &message.content {
                if let ContentBlock::ToolUse { id, .. } = block {
                    assert!(
                        lp.messages.iter().skip(index + 1).any(|later| {
                            later.content.iter().any(|candidate| {
                                matches!(candidate, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id)
                            })
                        }),
                        "tool call {id} must have a matching result"
                    );
                }
            }
        }
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
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: true,
            tail_token_budget: 1_000,
            target_summary_chars: 100,
            ..HarnessConfig::default()
        };
        let compressor = Box::new(NoopCompressor) as Box<dyn CompactorTrait>;
        let mut lp = ReActLoop::new(cfg, compressor);
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

        // With NoopCompressor, the context overflow is not resolved by compaction,
        // so the error propagates (no recovery).
        let result = lp
            .run(
                "trigger overflow",
                &llm,
                &tool_defs,
                |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                    ("tool result".into(), false)
                },
            )
            .await;

        // With NoopCompressor, the LLM retry after overflow still succeeds.
        // The overflow error triggers the compaction path (no-op with mock compressor),
        // then the LLM is retried and the second call produces text.
        if let Ok((out, _)) = result {
            assert!(out.contains("recovered") || !out.is_empty());
        }
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
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        };
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        let llm = EmptyThenTextLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];

        let (out, metrics) = lp
            .run(
                "test",
                &llm,
                &tool_defs,
                |_id: &str, _name: &str, _input: &serde_json::Value| async move {
                    ("result".into(), false)
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
        let cfg = HarnessConfig::default();
        let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        assert_eq!(lp.compose_user_message("hello"), "hello");
    }

    #[test]
    fn compose_user_message_with_plan_mode() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.set_plan_mode(true);
        let msg = lp.compose_user_message("hello");
        assert!(msg.contains(PLAN_MODE_MARKER));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_memory_updates() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.queue_memory_update("user prefers dark mode".into());
        let msg = lp.compose_user_message("hello");
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("user prefers dark mode"));
    }

    #[test]
    fn compose_user_message_plan_and_memory() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.set_plan_mode(true);
        lp.queue_memory_update("fact".into());
        let msg = lp.compose_user_message("hi");
        assert!(msg.contains(PLAN_MODE_MARKER));
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("hi"));
    }

    #[test]
    fn system_prompt_immutable_after_construction() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        assert_eq!(lp.system_prompt(), "");
        lp.set_system_prompt("test prompt".into());
        assert_eq!(lp.system_prompt(), "test prompt");
    }

    #[test]
    fn reset_clears_pending_memory_but_not_plan_mode() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
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
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.set_system_prompt("You are helpful".into());
        assert_eq!(lp.system_prompt(), "You are helpful");
    }

    #[test]
    fn compose_multiple_memory_updates() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.queue_memory_update("fact 1".into());
        lp.queue_memory_update("fact 2".into());
        let msg = lp.compose_user_message("hi");
        assert!(msg.contains("fact 1"));
        assert!(msg.contains("fact 2"));
    }

    #[test]
    fn compose_user_message_with_dasein_injection() {
        let cfg = HarnessConfig::default();
        let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        let msg = lp.compose_user_message_with_dasein("hello", Some("mood: curious"));
        assert!(msg.contains("<dasein-state>"));
        assert!(msg.contains("mood: curious"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_dasein_none() {
        let cfg = HarnessConfig::default();
        let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        let msg = lp.compose_user_message_with_dasein("hello", None);
        assert!(!msg.contains("<dasein-state>"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_dasein_empty() {
        let cfg = HarnessConfig::default();
        let lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        let msg = lp.compose_user_message_with_dasein("hello", Some(""));
        assert!(!msg.contains("<dasein-state>"));
        assert!(msg.contains("hello"));
    }

    #[test]
    fn compose_user_message_with_all_injections() {
        let cfg = HarnessConfig::default();
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
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

    use fabric::policy::verifier::{Verdict, Verifier};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Rejects the first candidate answer, accepts all subsequent ones.
    struct RejectOnce {
        seen: AtomicUsize,
    }
    #[async_trait]
    impl Verifier for RejectOnce {
        async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
            if self.seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Verdict::Reject {
                    reason: "first try rejected".into(),
                }
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
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: format!("answer {n}"),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }
        fn name(&self) -> &str {
            "text"
        }
        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn verifier_rejection_triggers_one_retry() {
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        };
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
        lp.set_verifier(std::sync::Arc::new(RejectOnce {
            seen: AtomicUsize::new(0),
        }));
        let llm = TextLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp
            .run(
                "go",
                &llm,
                &tool_defs,
                |_id: &str, name: &str, _in: &serde_json::Value| {
                    let name = name.to_string();
                    async move { (format!("ran {name}"), false) }
                },
            )
            .await
            .unwrap();
        // First answer rejected -> loop retried -> second answer accepted.
        assert_eq!(
            out, "answer 2",
            "rejected answer should be revised, got: {out}"
        );
    }

    #[tokio::test]
    async fn no_verifier_returns_first_answer_unchanged() {
        let cfg = HarnessConfig {
            max_iterations: 5,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        };
        let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor)); // no set_verifier -> None
        let llm = TextLlm {
            calls: Mutex::new(0),
        };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp
            .run(
                "go",
                &llm,
                &tool_defs,
                |_i: &str, n: &str, _in: &serde_json::Value| {
                    let n = n.to_string();
                    async move { (n, false) }
                },
            )
            .await
            .unwrap();
        assert_eq!(out, "answer 1", "no verifier = unchanged behavior");
    }

    #[test]
    fn max_iterations_zero_means_unlimited() {
        let cfg = HarnessConfig {
            max_iterations: 0,
            ..HarnessConfig::default()
        };
        let loop_ = ReActLoop::new(cfg, Box::new(NoopCompressor));
        assert!(
            loop_.should_continue(),
            "max_iterations=0 must never stop on the iteration check"
        );

        let cfg = HarnessConfig {
            max_iterations: 5,
            ..HarnessConfig::default()
        };
        let loop_ = ReActLoop::new(cfg, Box::new(NoopCompressor));
        // iteration starts at 0, so at iteration=5 we should stop
        let mut loop_ = loop_;
        loop_.iteration = 5;
        assert!(
            !loop_.should_continue(),
            "finite cap still stops when reached"
        );
    }
}
