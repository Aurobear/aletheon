//! Agent-level configuration: RuntimeConfig, AgentConfig, HooksConfig, PerceptionConfig,
//! AgentLoopConfig, CircuitBreakerConfig.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// RuntimeConfig — retained for orchestrator / react_loop backward compat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub max_iterations: usize,
    pub session_id: String,
    pub learning_enabled: bool,
    pub compaction_enabled: bool,
    pub tail_token_budget: usize,
    pub target_summary_chars: usize,
    pub context_window_tokens: usize,
    #[serde(default)]
    pub agent_loop: AgentLoopConfig,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            session_id: uuid::Uuid::new_v4().to_string(),
            learning_enabled: true,
            compaction_enabled: true,
            tail_token_budget: 16_000,
            target_summary_chars: 2_000,
            context_window_tokens: 128_000,
            agent_loop: AgentLoopConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// AgentConfig
// ---------------------------------------------------------------------------

/// Agent-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_true")]
    pub compaction_enabled: bool,
    #[serde(default = "default_compaction_keep_recent")]
    pub compaction_keep_recent: usize,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: usize,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_provider: None,
            default_model: None,
            max_iterations: default_max_iterations(),
            max_tokens: default_max_tokens(),
            compaction_enabled: true,
            compaction_keep_recent: default_compaction_keep_recent(),
            compaction_threshold: default_compaction_threshold(),
            system_prompt: default_system_prompt(),
        }
    }
}

pub(crate) fn default_max_iterations() -> usize {
    25
}
pub(crate) fn default_max_tokens() -> usize {
    100_000
}
pub(crate) fn default_true() -> bool {
    true
}
pub(crate) fn default_compaction_keep_recent() -> usize {
    10
}
pub(crate) fn default_compaction_threshold() -> usize {
    30
}

pub(crate) fn default_system_prompt() -> String {
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user."
        .to_string()
}

// ---------------------------------------------------------------------------
// HooksConfig
// ---------------------------------------------------------------------------

/// Hook script configuration.
///
/// Each field is a list of script paths to execute at the specified lifecycle point.
/// Paths may use `~` for home directory expansion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
// PerceptionConfig
// ---------------------------------------------------------------------------

/// Perception subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionConfig {
    /// Master switch. Off by default: the perception→behavior loop is not yet
    /// wired (see roadmap §T3). When false, no watchers are spawned.
    #[serde(default)]
    pub enabled: bool,
    /// Filesystem paths to watch with inotify.
    #[serde(default = "default_perception_watch_paths")]
    pub watch_paths: Vec<String>,
    /// Whether to enable journald log monitoring.
    #[serde(default = "default_true")]
    pub enable_journald: bool,
}

fn default_perception_watch_paths() -> Vec<String> {
    vec!["/etc".to_string(), "/var/log".to_string()]
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            watch_paths: default_perception_watch_paths(),
            enable_journald: true,
        }
    }
}

// ---------------------------------------------------------------------------
// EvolutionSettings
// ---------------------------------------------------------------------------

/// Self-evolution loop settings. Default OFF (HIGH-risk autonomy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSettings {
    /// Master switch for the self-evolution loop.
    /// When false (default), the loop is inert regardless of other settings.
    #[serde(default)] // bool default = false
    pub enabled: bool,
    /// Trigger evolution every N turns.
    #[serde(default = "default_evolution_trigger_every_n_turns")]
    pub trigger_every_n_turns: usize,
}

fn default_evolution_trigger_every_n_turns() -> usize {
    10
}

impl Default for EvolutionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger_every_n_turns: 10,
        }
    }
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
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: 0, // 0 = unlimited
            reflection_interval: 5,
            progress_check_interval: 3,
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
