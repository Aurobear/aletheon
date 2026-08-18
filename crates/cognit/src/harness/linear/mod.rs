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

pub use batching::{partition_tool_calls, stable_priority_order, ToolBatch};
pub use compaction_observability::{compaction_metrics, CompactionMetrics};
pub use metrics::TurnMetrics;

use compaction_observability::{record_degenerate, record_evicted, record_sampler_error};
use exploration::{
    exploration_input_token_budget, is_broad_discovery, is_repository_inspection,
    should_close_exploration,
};

/// Minimum length (trimmed chars) an answer must have before a
/// reflection-triggered stop is treated as terminal. Below this, both run loops
/// force one final tool-free synthesis pass instead of surfacing an empty/stub
/// answer, so reflection can halt tool use without discarding the turn.
pub(super) const MIN_SUBSTANTIVE_ANSWER_CHARS: usize = 40;

/// After this many consecutive tool failures, inject a one-shot "reassess and
/// try a different approach" nudge so the loop re-plans instead of repeating a
/// failing call. The counter resets on any tool success or after the nudge.
pub(super) const REPLAN_ON_CONSECUTIVE_ERRORS: usize = 3;

/// A host-required Agent runtime is an execution obligation, not a suggestion.
/// Six unrelated calls are enough to prove that the current strategy has
/// stalled; repeated stalls narrow the available actions and eventually stop.
pub(super) const REQUIRED_AGENT_STALL_CALLS: usize = 6;

/// Consecutive observation calls before the Host requires synthesis. A second
/// stall removes inspection capabilities for the rest of the turn.
pub(super) const REPOSITORY_INSPECTION_STALL_CALLS: usize = 12;

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
use crate::ports::verifier::Verifier;
use ::contracts::body::Action;
use ::contracts::compaction::CompactionStrategy;
use ::contracts::message::Message;
use ::contracts::{Clock, ToolDefinition};
use dasein::core::contracts::{Intent, IntentSource};
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
pub use ::contracts::compaction::CompactorTrait;

/// Async trait for planning capability batch execution order.
///
/// The planner receives the complete set of tool calls the LLM requested in
/// one iteration and returns a validated plan. Cognit applies the plan only
/// when the mode is `Enforce` and the plan is a valid exact permutation.
#[async_trait]
pub trait BatchPlanner: Send + Sync {
    async fn plan(
        &self,
        calls: Vec<::contracts::CapabilityCall>,
    ) -> anyhow::Result<::contracts::CapabilityBatchPlan>;
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
    /// Broad-discovery batches completed after the repository overview.
    /// Only batches containing at least one repository-wide scan (recursive
    /// glob, wildcard-directory glob, or un-scoped grep/file_search) count.
    /// Scoped/exact discovery calls do not consume this allowance.
    broad_discovery_batches: usize,
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
    /// Deterministic spawn/terminal progress for host-required Agent runtimes.
    required_agent_progress: Option<String>,
    required_agent_calls_without_progress: usize,
    required_agent_stalls: usize,
    repository_inspection_calls: usize,
    repository_inspection_stalls: usize,
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
            broad_discovery_batches: 0,
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
            required_agent_progress: None,
            required_agent_calls_without_progress: 0,
            required_agent_stalls: 0,
            repository_inspection_calls: 0,
            repository_inspection_stalls: 0,
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
        outcome: &::contracts::compaction::CompactionOutcome,
        event_sink: Option<&dyn crate::harness::event_sink::EventSink>,
    ) {
        match outcome.failure.as_ref() {
            Some(::contracts::compaction::CompactionFailure::DegenerateSummary { .. }) => {
                record_degenerate();
            }
            Some(::contracts::compaction::CompactionFailure::SamplerError { .. }) => {
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
        self.broad_discovery_batches = 0;
        self.consecutive_errors = 0;
        self.tool_budget.reset();
        self.circuit_breaker.reset();
        self.goal_tracker.reset();
        self.reflection_engine.reset();
        self.latest_completion_audit = None;
        self.required_agent_progress = None;
        self.required_agent_calls_without_progress = 0;
        self.required_agent_stalls = 0;
        self.repository_inspection_calls = 0;
        self.repository_inspection_stalls = 0;
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
        self.required_agent_progress = None;
        self.required_agent_calls_without_progress = 0;
        self.required_agent_stalls = 0;
        self.repository_inspection_calls = 0;
        self.repository_inspection_stalls = 0;
    }

    pub fn clear_cognitive_state(&mut self) {
        self.cognitive_state = None;
        self.evidence_ledger = EvidenceLedger::default();
        self.spawned_agents.clear();
        self.latest_completion_audit = None;
        self.required_agent_progress = None;
        self.required_agent_calls_without_progress = 0;
        self.required_agent_stalls = 0;
        self.repository_inspection_calls = 0;
        self.repository_inspection_stalls = 0;
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
    use serde_json::json;

    #[test]
    fn repository_inspection_classifier_covers_variable_search_and_read_calls() {
        for name in [
            "artifact_read",
            "code_graph",
            "file_read",
            "file_search",
            "glob",
            "grep",
            "repo_inspect",
            "tool_search",
        ] {
            assert!(is_repository_inspection(name), "missing {name}");
        }
        for name in ["agent_spawn", "apply_patch", "git_diff", "validation_run"] {
            assert!(!is_repository_inspection(name), "misclassified {name}");
        }
    }

    // --- typed input helpers ---
    fn broad_grep() -> serde_json::Value {
        json!({"pattern": "foo"})
    }
    fn scoped_grep() -> serde_json::Value {
        json!({"pattern": "foo", "path": "src/"})
    }
    fn broad_glob() -> serde_json::Value {
        json!({"pattern": "**/*.rs"})
    }
    fn wildcard_dir_glob() -> serde_json::Value {
        json!({"pattern": "crates/*/Cargo.toml"})
    }
    fn exact_glob() -> serde_json::Value {
        json!({"pattern": "docs/architecture.md"})
    }
    fn leaf_wildcard_glob() -> serde_json::Value {
        json!({"pattern": "schema/*.json"})
    }
    fn broad_file_search() -> serde_json::Value {
        json!({"query": "fn main"})
    }
    fn scoped_file_search() -> serde_json::Value {
        json!({"query": "fn main", "path": "crates/cognit/"})
    }
    fn file_read_input() -> serde_json::Value {
        json!({"path": "src/main.rs"})
    }
    fn shell_input() -> serde_json::Value {
        json!({"command": "ls"})
    }
    fn glob_root_absolute() -> serde_json::Value {
        json!({"root": "/tmp", "pattern": "*.rs"})
    }
    fn glob_root_parent() -> serde_json::Value {
        json!({"root": "../outside", "pattern": "*.rs"})
    }
    fn glob_root_scoped() -> serde_json::Value {
        json!({"root": "schema", "pattern": "*.json"})
    }
    fn glob_root_current_dir() -> serde_json::Value {
        json!({"root": ".", "pattern": "*.rs"})
    }
    fn glob_root_non_string() -> serde_json::Value {
        json!({"root": 42, "pattern": "*.rs"})
    }

    // --- is_broad_discovery unit tests ---

    #[test]
    fn exact_glob_is_scoped() {
        assert!(!is_broad_discovery("glob", &exact_glob()));
    }

    #[test]
    fn leaf_only_wildcard_glob_is_scoped() {
        assert!(!is_broad_discovery("glob", &leaf_wildcard_glob()));
    }

    #[test]
    fn recursive_glob_is_broad() {
        assert!(is_broad_discovery("glob", &broad_glob()));
    }

    #[test]
    fn wildcard_directory_glob_is_broad() {
        assert!(is_broad_discovery("glob", &wildcard_dir_glob()));
    }

    #[test]
    fn glob_with_absolute_root_is_broad() {
        assert!(is_broad_discovery("glob", &glob_root_absolute()));
    }

    #[test]
    fn glob_with_parent_root_is_broad() {
        assert!(is_broad_discovery("glob", &glob_root_parent()));
    }

    #[test]
    fn glob_with_non_string_root_is_broad() {
        assert!(is_broad_discovery("glob", &glob_root_non_string()));
    }

    #[test]
    fn glob_with_scoped_root_is_not_broad() {
        // Safe relative root + scoped leaf pattern → scoped.
        assert!(!is_broad_discovery("glob", &glob_root_scoped()));
    }

    #[test]
    fn glob_with_current_dir_root_is_not_broad() {
        // "." root is neutral; breadth is determined by pattern.
        assert!(!is_broad_discovery("glob", &glob_root_current_dir()));
    }

    #[test]
    fn grep_with_omitted_path_is_broad() {
        assert!(is_broad_discovery("grep", &broad_grep()));
    }

    #[test]
    fn grep_with_root_path_is_broad() {
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "foo", "path": "."})
        ));
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "foo", "path": "./"})
        ));
    }

    #[test]
    fn grep_with_safe_subpath_is_scoped() {
        assert!(!is_broad_discovery("grep", &scoped_grep()));
    }

    #[test]
    fn grep_with_path_traversal_is_broad() {
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "foo", "path": "../etc"})
        ));
    }

    #[test]
    fn grep_with_absolute_path_is_broad() {
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "foo", "path": "/tmp"})
        ));
    }

    #[test]
    fn file_search_omitted_path_is_broad() {
        assert!(is_broad_discovery("file_search", &broad_file_search()));
    }

    #[test]
    fn file_search_safe_subpath_is_scoped() {
        assert!(!is_broad_discovery("file_search", &scoped_file_search()));
    }

    // ── fail-closed path scope classification (audit 2025) ──

    #[test]
    fn absolute_tmp_path_is_broad() {
        // `/tmp/...` is absolute and must be broad, not scoped.
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "secret", "path": "/tmp/logs"})
        ));
        assert!(is_broad_discovery(
            "file_search",
            &json!({"query": "secret", "path": "/tmp/logs"})
        ));
    }

    #[test]
    fn parent_dir_component_is_broad() {
        // `../...` is a parent traversal and must be broad.
        assert!(is_broad_discovery(
            "grep",
            &json!({"pattern": "x", "path": "../outside"})
        ));
        assert!(is_broad_discovery(
            "file_search",
            &json!({"query": "x", "path": "sub/../../etc"})
        ));
    }

    #[test]
    fn double_dot_inside_filename_is_scoped() {
        // `src/foo..bar` is a safe normal filename containing two dots,
        // not a parent-dir component. Must remain scoped.
        assert!(!is_broad_discovery(
            "grep",
            &json!({"pattern": "x", "path": "src/foo..bar"})
        ));
        assert!(!is_broad_discovery(
            "file_search",
            &json!({"query": "x", "path": "src/foo..bar"})
        ));
        // Single-component filename with double dots is also scoped.
        assert!(!is_broad_discovery(
            "grep",
            &json!({"pattern": "x", "path": "foo..bar"})
        ));
    }

    #[test]
    fn glob_absolute_root_is_broad() {
        // `/etc/**` — absolute root must be broad before wildcard semantics.
        assert!(is_broad_discovery("glob", &json!({"pattern": "/etc/**"})));
        assert!(is_broad_discovery(
            "glob",
            &json!({"pattern": "/tmp/*.log"})
        ));
        // Exact absolute path without wildcards is still broad.
        assert!(is_broad_discovery(
            "glob",
            &json!({"pattern": "/etc/passwd"})
        ));
    }

    #[test]
    fn glob_parent_traversal_pattern_is_broad() {
        // `../*.rs` — parent-dir component must be broad.
        assert!(is_broad_discovery("glob", &json!({"pattern": "../*.rs"})));
        assert!(is_broad_discovery(
            "glob",
            &json!({"pattern": "src/../../*.toml"})
        ));
        // Patterns array with one unsafe entry makes the whole call broad.
        assert!(is_broad_discovery(
            "glob",
            &json!({"patterns": ["src/*.rs", "../escape.rs"]})
        ));
    }

    #[test]
    fn glob_safe_relative_root_remains_scoped() {
        // Leaf-only wildcard under a safe relative root is scoped.
        assert!(!is_broad_discovery("glob", &json!({"pattern": "src/*.rs"})));
        // Exact relative path is scoped.
        assert!(!is_broad_discovery(
            "glob",
            &json!({"pattern": "docs/architecture.md"})
        ));
        // Wildcard only in leaf position with a safe multi-segment root is scoped.
        assert!(!is_broad_discovery(
            "glob",
            &json!({"pattern": "crates/cognit/src/*.rs"})
        ));
    }

    #[test]
    fn file_read_is_not_discovery() {
        assert!(!is_broad_discovery("file_read", &file_read_input()));
    }

    #[test]
    fn shell_is_not_discovery() {
        assert!(!is_broad_discovery("shell", &shell_input()));
    }

    // --- should_close_exploration integration tests ---

    #[test]
    fn budget_scales_with_context_but_remains_bounded() {
        assert_eq!(exploration_input_token_budget(128_000), 10_000);
        assert_eq!(exploration_input_token_budget(1_000_000), 10_000);
        assert_eq!(exploration_input_token_budget(2_000_000), 20_000);
        assert_eq!(exploration_input_token_budget(10_000_000), 24_000);
    }

    #[test]
    fn closes_only_later_pure_inspection_batches_over_budget() {
        let bg = broad_grep();
        let bgl = broad_glob();
        let si = shell_input();
        let fr = file_read_input();

        // Iteration 1 never closes, regardless of tools.
        assert!(!should_close_exploration(
            1,
            20_000,
            1_000_000,
            false,
            0,
            [("grep", &bg)]
        ));
        // Under budget never closes.
        assert!(!should_close_exploration(
            2,
            9_999,
            1_000_000,
            false,
            0,
            [("grep", &bg), ("glob", &bgl)]
        ));
        // Later, over-budget, broad-scanning batch closes.
        assert!(should_close_exploration(
            2,
            10_000,
            1_000_000,
            false,
            0,
            [("grep", &bg), ("glob", &bgl)]
        ));
        // A batch containing a non-inspection tool never closes.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            [("grep", &bg), ("shell", &si)]
        ));
        // `file_read` is deliberately NOT inspection: reading a specific file is
        // always allowed and must not trip the exploration cutoff.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            [("file_read", &fr)]
        ));
        // Exact file reads remain available after repo_inspect.
        assert!(!should_close_exploration(
            3,
            50_000,
            1_000_000,
            true,
            1,
            [("file_read", &fr)]
        ));
        // After repo_inspect, broad-discovery batches over budget close at
        // the third batch (counter >= 2).
        assert!(!should_close_exploration(
            3,
            50_000,
            1_000_000,
            true,
            1,
            [("glob", &bgl), ("grep", &bg)]
        ));
        assert!(should_close_exploration(
            4,
            50_000,
            1_000_000,
            true,
            2,
            [("glob", &bgl), ("grep", &bg)]
        ));
        // Mixed batch with file_read never closes.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            false,
            0,
            [("file_read", &fr), ("glob", &bgl)]
        ));
    }

    #[test]
    fn repository_overview_keeps_exact_reads_and_allows_scoped_discovery() {
        let fr = file_read_input();
        let bgl = broad_glob();
        let sg = exact_glob();
        let bfs = broad_file_search();
        let si = shell_input();

        // file_read after repo_inspect never closes.
        assert!(!should_close_exploration(
            2,
            20_000,
            1_000_000,
            true,
            0,
            [("file_read", &fr)]
        ));
        // A broad glob batch after repo_inspect + 0 prior is still allowed.
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            0,
            [("glob", &bgl)]
        ));
        // After 1 prior broad batch, a second broad batch is still allowed.
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            1,
            [("glob", &bgl)]
        ));
        // A third broad-discovery batch over budget closes.
        assert!(should_close_exploration(
            4,
            20_000,
            1_000_000,
            true,
            2,
            [("file_search", &bfs)]
        ));
        // Mixed batch with file_read never closes.
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            0,
            [("file_read", &fr), ("glob", &bgl)]
        ));
        // Non-discovery tool in batch prevents closure.
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            2,
            [("file_read", &fr), ("shell", &si)]
        ));
        // Scoped glob batches do NOT close, even after prior broad batches.
        // They do not consume the broad counter.
        assert!(!should_close_exploration(
            5,
            20_000,
            1_000_000,
            true,
            3,
            [("glob", &sg)]
        ));
        // Scoped grep with explicit subpath does NOT close.
        let sgrep = scoped_grep();
        assert!(!should_close_exploration(
            5,
            20_000,
            1_000_000,
            true,
            3,
            [("grep", &sgrep)]
        ));
    }

    #[test]
    fn file_read_with_broad_glob_does_not_evade_counter() {
        // When tool_exec increments the counter after a mixed batch
        // [file_read, broad_glob], the batch itself does NOT close
        // (file_read is not an inspection tool), but the counter
        // advances. A subsequent pure broad-inspection batch then
        // closes at counter >= 2.
        let fr = file_read_input();
        let bgl = broad_glob();

        // Mixed batch with file_read never closes (all_inspection is false).
        assert!(!should_close_exploration(
            3,
            20_000,
            1_000_000,
            true,
            1, // counter was already incremented by tool_exec for this batch
            [("file_read", &fr), ("glob", &bgl)]
        ));
        // But the counter still advanced to 2 after the mixed batch,
        // so the next pure broad-inspection batch closes.
        assert!(should_close_exploration(
            4,
            20_000,
            1_000_000,
            true,
            2,
            [("glob", &bgl)]
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
