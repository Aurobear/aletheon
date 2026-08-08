pub mod awareness;
pub mod batching;
pub mod circuit_breaker;
mod compaction_observability;
mod completion;
mod exploration;
pub mod goal_tracker;
pub mod message_compose;
pub mod metrics;
pub mod reflection;
mod step;
pub mod tool_budget;
mod tool_exec;
mod tool_output;

pub use batching::{partition_tool_calls, ToolBatch};
pub use compaction_observability::{compaction_metrics, CompactionMetrics};
pub use metrics::TurnMetrics;

use compaction_observability::{record_degenerate, record_evicted, record_sampler_error};
use exploration::{exploration_input_token_budget, should_close_exploration};

/// Minimum length (trimmed chars) an answer must have before a
/// reflection-triggered stop is treated as terminal. Below this, both run loops
/// force one final tool-free synthesis pass instead of surfacing an empty/stub
/// answer, so reflection can halt tool use without discarding the turn.
pub(super) const MIN_SUBSTANTIVE_ANSWER_CHARS: usize = 40;

/// After this many consecutive tool failures, inject a one-shot "reassess and
/// try a different approach" nudge so the loop re-plans instead of repeating a
/// failing call. The counter resets on any tool success or after the nudge.
pub(super) const REPLAN_ON_CONSECUTIVE_ERRORS: usize = 3;

use async_trait::async_trait;
use circuit_breaker::CircuitBreaker;
use goal_tracker::GoalTracker;
use reflection::ReflectionEngine;
use tool_budget::ToolBudget;

use crate::adapters::inference::provider::{LlmProvider, LlmResponse, LlmStream};
use crate::core::awareness_signal::AwarenessSignal;
use crate::core::{
    AgentRuntimeId, CognitiveTurnState, CompletionGateMode, EvidenceLedger, ProgressDecision,
};
use crate::harness::config::HarnessConfig;
use crate::harness::interrupt::InterruptFlag;
use fabric::body::Action;
use fabric::message::Message;
use fabric::policy::verifier::Verifier;
use fabric::self_field::{Intent, IntentSource};
use fabric::{Clock, CompactionStrategy, ToolDefinition};
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

pub type EvictedCallback = Arc<dyn Fn(Vec<Message>) + Send + Sync>;

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
    /// Provider input tokens spent during the current turn. This is a billed
    /// work budget, distinct from active context occupancy.
    turn_input_tokens: u64,
    /// A successful typed repository overview result was observed this turn.
    repository_context_seen: bool,
    /// Exact read/discovery batches completed after the repository overview.
    repository_followup_read_batches: usize,
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
    /// Bounded handoff for messages removed by guarded compaction. None is the
    /// documented no-op until a Mnemosyne promoter is composed.
    evicted_callback: Option<EvictedCallback>,
    /// Clock for deterministic time (mono/wall).
    clock: Arc<dyn Clock>,
    /// Typed task state is deliberately separate from compactable model messages.
    cognitive_state: Option<CognitiveTurnState>,
    /// Authoritative adapter-produced evidence for the active task.
    evidence_ledger: EvidenceLedger,
    /// Agent instances spawned during the active turn. A terminal wait may
    /// satisfy a required-agent obligation only for one of these instances;
    /// stale receipts from earlier turns are deliberately ineligible.
    spawned_agents: std::collections::BTreeMap<String, AgentRuntimeId>,
    /// Latest deterministic completion audit. Initially observed in shadow mode.
    latest_completion_audit: Option<ProgressDecision>,
    completion_gate_mode: CompletionGateMode,
    max_completion_retries: u32,
    grounded_outcome_sink: Option<Arc<dyn crate::core::GroundedOutcomeSink>>,
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
            turn_input_tokens: 0,
            repository_context_seen: false,
            repository_followup_read_batches: 0,
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
            evicted_callback: None,
            clock,
            cognitive_state: None,
            evidence_ledger: EvidenceLedger::default(),
            spawned_agents: std::collections::BTreeMap::new(),
            latest_completion_audit: None,
            completion_gate_mode: CompletionGateMode::Shadow,
            max_completion_retries: 2,
            grounded_outcome_sink: None,
        }
    }

    #[cfg(test)]
    pub fn new(config: HarnessConfig, compressor: Box<dyn CompactorTrait>) -> Self {
        Self::new_with_clock(
            config,
            compressor,
            Arc::new(kernel::chronos::TestClock::default()),
        )
    }

    /// Proactive best-effort compaction. Routes to the guarded
    /// `maybe_compact_v2` when `grok_hardening.compaction_v2` is set (a
    /// degenerate summary or sampler failure then leaves the buffer unchanged,
    /// logged), otherwise the legacy `maybe_compact`. Errors are swallowed: a
    /// proactive pass must never fail the turn.
    async fn run_proactive_compaction(
        &mut self,
        llm: &dyn LlmProvider,
        event_sink: Option<&dyn crate::harness::event_sink::EventSink>,
    ) {
        if self.config.compaction_v2 {
            match self
                .compressor
                .maybe_compact_v2(&mut self.messages, llm, CompactionStrategy::TailKeep)
                .await
            {
                Ok(outcome) => {
                    self.observe_compaction_outcome(&outcome, event_sink);
                    if let Some(failure) = &outcome.failure {
                        tracing::warn!(
                            ?failure,
                            "compaction_v2 skipped (fail-safe, context preserved)"
                        );
                    }
                }
                Err(e) => tracing::warn!("compaction_v2 error (context preserved): {e}"),
            }
        } else {
            let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
        }
    }

    /// Reactive compaction on a context-overflow error. Same v2/legacy routing
    /// as the proactive path, but errors propagate so the caller can decide
    /// whether the retried completion is still viable.
    async fn run_reactive_compaction(
        &mut self,
        llm: &dyn LlmProvider,
        event_sink: Option<&dyn crate::harness::event_sink::EventSink>,
    ) -> anyhow::Result<()> {
        if self.config.compaction_v2 {
            let outcome = self
                .compressor
                .maybe_compact_v2(&mut self.messages, llm, CompactionStrategy::TailKeep)
                .await?;
            self.observe_compaction_outcome(&outcome, event_sink);
            if let Some(failure) = &outcome.failure {
                tracing::warn!(
                    ?failure,
                    "compaction_v2 could not reduce context (fail-safe)"
                );
            }
        } else {
            self.compressor
                .maybe_compact(&mut self.messages, llm)
                .await?;
        }
        Ok(())
    }

    fn observe_compaction_outcome(
        &self,
        outcome: &fabric::CompactionOutcome,
        event_sink: Option<&dyn crate::harness::event_sink::EventSink>,
    ) {
        match outcome.failure.as_ref() {
            Some(fabric::CompactionFailure::DegenerateSummary { .. }) => {
                record_degenerate();
            }
            Some(fabric::CompactionFailure::SamplerError { .. }) => {
                record_sampler_error();
            }
            _ => {}
        }
        record_evicted(outcome.evicted.len());
        if let Some(event_sink) = event_sink {
            event_sink.emit(crate::harness::event_sink::Event::CompactionOutcome {
                strategy: format!("{:?}", outcome.strategy).to_ascii_lowercase(),
                applied: outcome.applied,
                tokens_before: outcome.tokens_before,
                tokens_after: outcome.tokens_after,
                evicted_messages: outcome.evicted.len(),
                failure: outcome
                    .failure
                    .as_ref()
                    .map(|failure| format!("{failure:?}")),
            });
        }
        if !outcome.evicted.is_empty() {
            if let Some(callback) = &self.evicted_callback {
                callback(outcome.evicted.clone());
            }
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

    pub fn clock_handle(&self) -> Arc<dyn Clock> {
        self.clock.clone()
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
        self.turn_input_tokens = 0;
        self.repository_context_seen = false;
        self.repository_followup_read_batches = 0;
        self.consecutive_errors = 0;
        self.tool_budget.reset();
        self.circuit_breaker.reset();
        self.goal_tracker.reset();
        self.reflection_engine.reset();
        self.latest_completion_audit = None;
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

    /// Install typed task state for completion auditing. Evidence is retained
    /// across inference iterations and message compaction within the task.
    pub fn set_cognitive_state(&mut self, state: CognitiveTurnState) {
        self.cognitive_state = Some(state);
        self.evidence_ledger = EvidenceLedger::default();
        self.spawned_agents.clear();
        self.latest_completion_audit = None;
    }

    pub fn clear_cognitive_state(&mut self) {
        self.cognitive_state = None;
        self.evidence_ledger = EvidenceLedger::default();
        self.spawned_agents.clear();
        self.latest_completion_audit = None;
        self.completion_gate_mode = CompletionGateMode::Shadow;
    }

    pub fn set_completion_gate_mode(&mut self, mode: CompletionGateMode) {
        self.completion_gate_mode = mode;
    }

    pub fn set_grounded_outcome_sink(&mut self, sink: Arc<dyn crate::core::GroundedOutcomeSink>) {
        self.grounded_outcome_sink = Some(sink);
    }

    pub fn evidence_ledger_mut(&mut self) -> &mut EvidenceLedger {
        &mut self.evidence_ledger
    }

    pub fn latest_completion_audit(&self) -> Option<&ProgressDecision> {
        self.latest_completion_audit.as_ref()
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

    /// Install the bounded evicted-message promoter. The callback receives one
    /// owned batch after the buffer mutation has succeeded.
    pub fn set_evicted_callback(&mut self, callback: EvictedCallback) {
        self.evicted_callback = Some(callback);
    }
}

#[cfg(test)]
mod exploration_budget_tests {
    use super::*;

    #[test]
    fn budget_scales_with_context_but_remains_bounded() {
        assert_eq!(exploration_input_token_budget(128_000), 10_000);
        assert_eq!(exploration_input_token_budget(1_000_000), 10_000);
        assert_eq!(exploration_input_token_budget(2_000_000), 20_000);
        assert_eq!(exploration_input_token_budget(10_000_000), 24_000);
    }

    #[test]
    fn closes_only_later_pure_inspection_batches_over_budget() {
        // Iteration 1 never closes, regardless of tools.
        assert!(!should_close_exploration(
            1,
            20_000,
            1_000_000,
            false,
            0,
            ["grep"]
        ));
        // Under budget never closes.
        assert!(!should_close_exploration(
            2,
            9_999,
            1_000_000,
            false,
            0,
            ["grep", "glob"]
        ));
        // Later, over-budget, pure breadth-scanning batch closes.
        assert!(should_close_exploration(
            2,
            10_000,
            1_000_000,
            false,
            0,
            ["grep", "glob"]
        ));
        // A batch containing a non-inspection tool never closes.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            ["grep", "shell"]
        ));
        // `file_read` is deliberately NOT inspection: reading a specific file is
        // always allowed and must not trip the exploration cutoff, even when a
        // batch is otherwise pure breadth-scanning and over budget.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            ["file_read"]
        ));
        // Exact file reads remain available after repo_inspect even when an
        // earlier exact batch was truncated. The breadth guard must not turn a
        // coding task into synthesis before the named edit targets are read.
        assert!(!should_close_exploration(
            3,
            50_000,
            1_000_000,
            true,
            1,
            ["file_read"]
        ));
        // Repeated discovery after repo_inspect still closes once over budget.
        assert!(should_close_exploration(
            3,
            50_000,
            1_000_000,
            true,
            1,
            ["glob", "grep"]
        ));
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            ["file_read", "glob"]
        ));
    }

    #[test]
    fn repository_overview_keeps_exact_reads_but_closes_repeated_discovery() {
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            true,
            0,
            ["file_read"]
        ));
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            1,
            ["file_read"]
        ));
        assert!(should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            1,
            ["file_search"]
        ));
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            1,
            ["file_read", "shell"]
        ));
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
mod tests;
