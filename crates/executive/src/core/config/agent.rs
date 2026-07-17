//! Agent-level configuration: ExecutiveConfig, AgentConfig, HooksConfig, PerceptionConfig,
//! AgentLoopConfig, CircuitBreakerConfig.
//!
//! AgentConfig, PerceptionConfig, and EvolutionSettings are re-exported from aletheon-cognit.
//! ExecutiveConfig, HooksConfig, AgentLoopConfig, CircuitBreakerConfig remain executive-specific.

use cognit::harness::HarnessKind;
use serde::{Deserialize, Serialize};

// Re-exports from cognit to avoid duplication.
pub use cognit::config::AgentConfig;
pub use cognit::config::EvolutionSettings;
pub use cognit::config::PerceptionConfig;

// ---------------------------------------------------------------------------
// ExecutiveConfig — retained for orchestrator / react_loop backward compat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveConfig {
    pub max_iterations: usize,
    pub session_id: String,
    pub learning_enabled: bool,
    pub compaction_enabled: bool,
    pub tail_token_budget: usize,
    pub target_summary_chars: usize,
    pub context_window_tokens: usize,
    #[serde(default)]
    pub conscious_arbitration_mode: fabric::ConsciousArbitrationMode,
    #[serde(default)]
    pub agent_loop: AgentLoopConfig,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    /// Which cognitive harness implementation to construct (see
    /// the configured cognitive harness. Defaults to `Linear`.
    /// preserving current behavior. TOML key: `harness_kind = "linear"`.
    #[serde(default)]
    pub harness_kind: HarnessKind,
}

impl Default for ExecutiveConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            session_id: uuid::Uuid::new_v4().to_string(),
            learning_enabled: true,
            compaction_enabled: true,
            tail_token_budget: 16_000,
            target_summary_chars: 2_000,
            context_window_tokens: 128_000,
            conscious_arbitration_mode: fabric::ConsciousArbitrationMode::Observe,
            agent_loop: AgentLoopConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
            harness_kind: HarnessKind::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// HooksConfig
// ---------------------------------------------------------------------------

/// Hook script configuration.
///
/// Each field is a list of script paths to execute at the specified lifecycle point.
/// Paths may use `~` for home directory expansion.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HooksConfig {
    /// Scripts to run before each turn (receives user prompt as JSON on stdin).
    #[serde(default)]
    pub pre_turn: Vec<String>,
    /// Scripts to run after each tool call (receives tool name + result as JSON on stdin).
    #[serde(default)]
    pub post_tool: Vec<String>,
    /// Scripts to run on session end (receives session_id + cwd as JSON on stdin).
    #[serde(default)]
    pub on_session_end: Vec<String>,
    /// Scripts to run before each tool call (can block execution).
    #[serde(default)]
    pub pre_tool: Vec<String>,
}

// ---------------------------------------------------------------------------
// AgentLoopConfig
// ---------------------------------------------------------------------------

/// Agent loop configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopConfig {
    /// Maximum tool calls per turn.
    pub max_tool_calls: usize,
    /// Reflection interval (every N tool calls).
    pub reflection_interval: usize,
    /// Progress check interval (every N tool calls).
    pub progress_check_interval: usize,
    /// Maximum tool calls before reflection recommends stopping.
    pub reflection_tool_call_limit: usize,
    /// Storm breaker: consecutive identical failures before warning.
    pub storm_breaker_failure_threshold: usize,
    /// Storm breaker: consecutive successes before warning (higher to reduce noise).
    pub storm_breaker_success_threshold: usize,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: 0, // 0 = unlimited
            reflection_interval: 5,
            progress_check_interval: 3,
            reflection_tool_call_limit: 100,
            storm_breaker_failure_threshold: 3,
            storm_breaker_success_threshold: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// CircuitBreakerConfig
// ---------------------------------------------------------------------------

/// Circuit breaker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Maximum repeated calls before tripping.
    pub max_repeats: usize,
    /// Window size for tracking recent calls.
    pub window_size: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            max_repeats: 3,
            window_size: 10,
        }
    }
}
