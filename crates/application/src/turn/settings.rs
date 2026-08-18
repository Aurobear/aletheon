//! Application-owned immutable settings required by one turn.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRuntimeSettings {
    pub max_iterations: usize,
    pub learning_enabled: bool,
    pub compaction_enabled: bool,
    pub compaction_v2: bool,
    pub compaction_threshold_percent: usize,
    pub streaming_tools: bool,
    pub tail_token_budget: usize,
    pub target_summary_chars: usize,
    pub context_window_tokens: usize,
    pub max_tool_calls: usize,
    pub reflection_interval: usize,
    pub reflection_tool_call_limit: usize,
    pub circuit_breaker_max_repeats: usize,
    pub circuit_breaker_window_size: usize,
    pub multi_agent_enabled: bool,
    pub automatic_multi_agent_for_coding: bool,
    pub max_agent_depth: u16,
}
use std::collections::HashSet;

/// Immutable authorization and behavior snapshot resolved once per turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTurnProfile {
    pub profile_name: String,
    pub allowed_tools: HashSet<String>,
    pub delegated_tools: HashSet<String>,
    pub system_prompt: String,
    pub model_policy: Option<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub tool_schema_tokens: contracts::ContextCostTokens,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub approval_policy: contracts::AgentApprovalPolicy,
    pub tool_timeout_ms: u64,
}

/// Immutable admission and durability policy consumed by the Turn
/// coordinator. Host configuration is normalized into this snapshot during
/// composition so the use case never depends on a binary-owned config type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCoordinatorSettings {
    pub prompt_queue: bool,
    pub compaction_v2: bool,
    pub backpressure: runtime::backpressure::BackpressureConfig,
}

impl Default for TurnCoordinatorSettings {
    fn default() -> Self {
        Self {
            prompt_queue: false,
            compaction_v2: false,
            backpressure: runtime::backpressure::BackpressureConfig::default(),
        }
    }
}
