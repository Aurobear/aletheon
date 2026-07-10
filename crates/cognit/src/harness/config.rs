//! Harness configuration — extracted subset of RuntimeConfig used by ReActLoop.
//!
//! Lives in the cognit crate to avoid a circular dependency (runtime → cognit).
//! The orchestrator converts RuntimeConfig → HarnessConfig when creating the harness.

/// Configuration for a cognitive harness (e.g. ReActLoop).
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    pub max_iterations: usize,
    pub compaction_enabled: bool,
    pub tail_token_budget: usize,
    pub target_summary_chars: usize,
    pub context_window_tokens: usize,
    pub max_tool_calls: usize,
    pub reflection_interval: usize,
    pub reflection_tool_call_limit: usize,
    pub circuit_breaker_max_repeats: usize,
    pub circuit_breaker_window_size: usize,
    pub learning_enabled: bool,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            compaction_enabled: true,
            tail_token_budget: 16_000,
            target_summary_chars: 2_000,
            context_window_tokens: 128_000,
            max_tool_calls: 20,
            reflection_interval: 5,
            reflection_tool_call_limit: 3,
            circuit_breaker_max_repeats: 5,
            circuit_breaker_window_size: 10,
            learning_enabled: true,
        }
    }
}
