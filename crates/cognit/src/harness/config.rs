//! Harness configuration — extracted subset of ExecutiveConfig used by ReActLoop.
//!
//! Lives in the cognit crate to avoid a circular dependency (runtime → cognit).
//! The orchestrator converts ExecutiveConfig → HarnessConfig when creating the harness.

/// Configuration for a cognitive harness (e.g. ReActLoop).
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    pub max_iterations: usize,
    pub compaction_enabled: bool,
    /// Fraction of the context window (as a whole percent, e.g. `80` = 80%) at
    /// which automatic compaction triggers. Wired into the compressor threshold;
    /// `80` preserves the historical hardcoded `0.8` behavior.
    pub compaction_threshold_percent: usize,
    pub tail_token_budget: usize,
    pub target_summary_chars: usize,
    pub context_window_tokens: usize,
    pub max_tool_calls: usize,
    pub reflection_interval: usize,
    pub reflection_tool_call_limit: usize,
    pub circuit_breaker_max_repeats: usize,
    pub circuit_breaker_window_size: usize,
    pub learning_enabled: bool,
    /// C1: when true, compaction uses the guarded `maybe_compact_v2` (degenerate
    /// / failure leaves the buffer unchanged). Default false = legacy
    /// `maybe_compact`. Set from `grok_hardening.compaction_v2`.
    pub compaction_v2: bool,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            compaction_enabled: true,
            compaction_threshold_percent: 80,
            tail_token_budget: 16_000,
            target_summary_chars: 2_000,
            context_window_tokens: 128_000,
            // Claude-style: do NOT cap tool use at a small fixed count. `0` =
            // unlimited (honored by ToolBudget); the turn is bounded by
            // `max_iterations` and the context window instead of a tiny per-turn
            // ceiling. This matches the daemon's AgentLoopConfig default (0) and
            // avoids marking a complete-but-thorough turn as failed just because
            // it read many files.
            max_tool_calls: 0,
            reflection_interval: 5,
            // Advisory reflection ceiling. Must stay positive (0 would stop
            // immediately). Aligned with the daemon's AgentLoopConfig (100) so a
            // deep, legitimate task is not halted mid-way after a handful of
            // tool calls.
            reflection_tool_call_limit: 100,
            circuit_breaker_max_repeats: 5,
            circuit_breaker_window_size: 10,
            learning_enabled: true,
            compaction_v2: false,
        }
    }
}
