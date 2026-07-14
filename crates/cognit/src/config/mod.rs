//! Configuration types shared between brain-core and runtime.
//!
//! These types were originally in the core crate, then moved to aletheon-runtime.
//! Duplicated here to break the cyclic dependency (brain-core <-> runtime).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Dynamic model routing — maps task types to model specs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRoutingConfig {
    /// Default model for general chat (e.g., "mimo/mimo-v2.5-pro").
    pub default: Option<String>,
    /// Model for multimodal inputs (images, audio).
    pub multimodal: Option<String>,
    /// Cheap model for simple tasks, code reading, extraction.
    pub cheap: Option<String>,
    /// Model for complex reasoning tasks.
    pub reasoning: Option<String>,
    /// Model for AutoMemory fact extraction.
    pub auto_memory: Option<String>,
}

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    #[serde(default)]
    pub model_routing: ModelRoutingConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub perception: PerceptionConfig,
    #[serde(default)]
    pub evolution: EvolutionSettings,
    #[serde(default)]
    pub telegram: TelegramConfig,
}

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

/// 0 means "no iteration cap" — termination then relies on the LLM stopping,
/// the circuit breaker, repeated-call detection, and the tool budget.
fn default_max_iterations() -> usize {
    0
}
fn default_max_tokens() -> usize {
    100_000
}
fn default_true() -> bool {
    true
}
fn default_compaction_keep_recent() -> usize {
    10
}
fn default_compaction_threshold() -> usize {
    30
}

fn default_system_prompt() -> String {
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user. \
     Before stating any conclusion about your own runtime state, logs, or configuration, \
     you MUST read the actual logs and the actually-effective config file first — never guess \
     or invent an explanation."
        .to_string()
}

/// Wire protocol between client and LLM server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Transport {
    Openai,
    Anthropic,
    #[default]
    Auto,
}

/// Per-provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub models: Vec<String>,
    /// Override the default max context length for this provider's models.
    /// If not set, the provider uses its built-in default (128K for OpenAI, 200K for Anthropic).
    #[serde(default)]
    pub max_context_length: Option<usize>,
    /// Optional static pricing for per-provider cost accounting. `None` = unpriced.
    #[serde(default)]
    pub pricing: Option<ProviderPricing>,
}

/// Optional static per-provider pricing (USD per 1K tokens) for cost accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPricing {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
}

// ---------------------------------------------------------------------------
// New config sub-structs
// ---------------------------------------------------------------------------

/// Sandbox execution preference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// "auto", "require", or "forbid"
    #[serde(default = "default_sandbox_preference")]
    pub preference: String,
    #[serde(default)]
    pub bubblewrap_path: Option<String>,
}

fn default_sandbox_preference() -> String {
    "auto".to_string()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            preference: default_sandbox_preference(),
            bubblewrap_path: None,
        }
    }
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// "stdio", "http", or "sse"
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// For stdio transport: command to run
    #[serde(default)]
    pub command: Option<String>,
    /// For http/sse transport: URL to connect to
    #[serde(default)]
    pub url: Option<String>,
    /// Environment variable containing the bearer token (never the token itself).
    #[serde(default)]
    pub bearer_token_env: Option<String>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: default_mcp_transport(),
            command: None,
            url: None,
            bearer_token_env: None,
        }
    }
}

/// Plugin directories.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsConfig {
    #[serde(default)]
    pub directories: Vec<String>,
}

/// Memory backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// "sqlite" or "in_memory"
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
    /// Optional gbrain shared memory integration (disabled by default).
    #[serde(default)]
    pub gbrain: GbrainMemoryConfig,
}

fn default_memory_backend() -> String {
    "sqlite".to_string()
}
fn default_memory_data_dir() -> String {
    "~/.aletheon/memory".to_string()
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
            data_dir: default_memory_data_dir(),
            gbrain: GbrainMemoryConfig::default(),
        }
    }
}
/// gbrain shared memory integration configuration.
///
/// Disabled by default. When enabled, the daemon connects to a gbrain
/// MCP server at startup and injects recalled content into dynamic turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GbrainMemoryConfig {
    /// Master switch. When false (default), gbrain is not connected.
    #[serde(default)]
    pub enabled: bool,
    /// MCP server name this gbrain instance is registered under.
    #[serde(default = "default_gbrain_server_name")]
    pub server_name: String,
    /// Primary source identifier for this project.
    #[serde(default = "default_gbrain_source")]
    pub source: String,
    /// Secondary (general) source identifier.
    #[serde(default = "default_general_source")]
    pub general_source: String,
    /// Recall timeout in milliseconds.
    #[serde(default = "default_gbrain_timeout_ms")]
    pub timeout_ms: u64,
    /// Maximum number of recalled results.
    #[serde(default = "default_gbrain_max_results")]
    pub max_results: usize,
    /// Maximum characters in rendered recall block.
    #[serde(default = "default_gbrain_max_chars")]
    pub max_chars: usize,
    /// Enable durable outbox capture (disabled by default).
    #[serde(default)]
    pub capture_enabled: bool,
    /// Directory for durable outbox entries.
    #[serde(default = "default_gbrain_outbox_dir")]
    pub outbox_dir: String,
}

fn default_gbrain_server_name() -> String {
    "gbrain".to_string()
}
fn default_gbrain_source() -> String {
    "aletheon".to_string()
}
fn default_general_source() -> String {
    "general".to_string()
}
fn default_gbrain_timeout_ms() -> u64 {
    1200
}
fn default_gbrain_max_results() -> usize {
    4
}
fn default_gbrain_max_chars() -> usize {
    6000
}
fn default_gbrain_outbox_dir() -> String {
    "~/.aletheon/gbrain-outbox".to_string()
}

impl Default for GbrainMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: default_gbrain_server_name(),
            source: default_gbrain_source(),
            general_source: default_general_source(),
            timeout_ms: default_gbrain_timeout_ms(),
            max_results: default_gbrain_max_results(),
            max_chars: default_gbrain_max_chars(),
            capture_enabled: false,
            outbox_dir: default_gbrain_outbox_dir(),
        }
    }
}

/// Daemon runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_socket_path")]
    pub socket_path: String,
    #[serde(default = "default_daemon_log_level")]
    pub log_level: String,
}

fn default_daemon_socket_path() -> String {
    "/run/aletheond/aletheond.sock".to_string()
}
fn default_daemon_log_level() -> String {
    "info".to_string()
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: default_daemon_socket_path(),
            log_level: default_daemon_log_level(),
        }
    }
}

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
            trigger_every_n_turns: default_evolution_trigger_every_n_turns(),
        }
    }
}

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

/// Telegram bot configuration for owner-only control channel.
///
/// The config stores the environment-variable NAME, never the token value itself.
/// The runtime reads the token from that env var at startup.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TelegramConfig {
    /// Master switch. When false (default), the Telegram bot is not started.
    #[serde(default)]
    pub enabled: bool,
    /// Environment variable name that holds the bot token.
    /// Example value: `"ALETHEON_TELEGRAM_BOT_TOKEN"`.
    pub bot_token_env: Option<String>,
    /// Owner's Telegram user ID. Only messages from this user are accepted.
    pub owner_user_id: Option<i64>,
    /// Polling timeout in seconds (clamped to 1–50).
    #[serde(default = "default_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}

fn default_poll_timeout_secs() -> u64 {
    10
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token_env: None,
            owner_user_id: None,
            poll_timeout_secs: default_poll_timeout_secs(),
        }
    }
}

impl TelegramConfig {
    /// Validate the configuration, returning a list of errors.
    /// Also clamps `poll_timeout_secs` to the allowed 1–50 range.
    pub fn validate(&mut self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // Clamp poll timeout
        self.poll_timeout_secs = self.poll_timeout_secs.clamp(1, 50);

        if !self.enabled {
            // Disabled: no token or owner required
            return errors;
        }

        // Enabled: require bot_token_env
        match &self.bot_token_env {
            None => {
                errors.push("telegram.enabled=true but bot_token_env is not set".to_string());
            }
            Some(name) if name.trim().is_empty() => {
                errors.push("telegram.enabled=true but bot_token_env is empty".to_string());
            }
            Some(_) => {}
        }

        // Enabled: require owner_user_id > 0
        match self.owner_user_id {
            None => {
                errors.push("telegram.enabled=true but owner_user_id is not set".to_string());
            }
            Some(id) if id <= 0 => {
                errors.push(format!(
                    "telegram.enabled=true but owner_user_id={} is not positive",
                    id
                ));
            }
            Some(_) => {}
        }

        errors
    }
}

// ---------------------------------------------------------------------------
// AppConfig methods
// ---------------------------------------------------------------------------

impl AppConfig {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::from_file(path).unwrap_or_default()
    }

    /// Merge `other` into `self`. Fields in `other` that are non-default
    /// override `self`. Lists are appended (providers merged by name).
    pub fn merge(&mut self, other: AppConfig) {
        // Agent: override non-default values
        if other.agent.default_provider.is_some() {
            self.agent.default_provider = other.agent.default_provider;
        }
        if other.agent.default_model.is_some() {
            self.agent.default_model = other.agent.default_model;
        }
        if other.agent.max_iterations != default_max_iterations() {
            self.agent.max_iterations = other.agent.max_iterations;
        }
        if other.agent.max_tokens != default_max_tokens() {
            self.agent.max_tokens = other.agent.max_tokens;
        }
        if other.agent.compaction_keep_recent != default_compaction_keep_recent() {
            self.agent.compaction_keep_recent = other.agent.compaction_keep_recent;
        }
        if other.agent.compaction_threshold != default_compaction_threshold() {
            self.agent.compaction_threshold = other.agent.compaction_threshold;
        }
        if other.agent.system_prompt != default_system_prompt() {
            self.agent.system_prompt = other.agent.system_prompt;
        }
        if !other.agent.compaction_enabled {
            self.agent.compaction_enabled = other.agent.compaction_enabled;
        }

        // Providers: merge by name, append new ones
        for other_provider in other.providers {
            if let Some(existing) = self
                .providers
                .iter_mut()
                .find(|p| p.name == other_provider.name)
            {
                *existing = other_provider;
            } else {
                self.providers.push(other_provider);
            }
        }

        // Model aliases: merge (other wins)
        for (key, value) in other.model_aliases {
            self.model_aliases.insert(key, value);
        }

        // Model routing: override non-default values
        if other.model_routing.default.is_some() {
            self.model_routing.default = other.model_routing.default;
        }
        if other.model_routing.multimodal.is_some() {
            self.model_routing.multimodal = other.model_routing.multimodal;
        }
        if other.model_routing.cheap.is_some() {
            self.model_routing.cheap = other.model_routing.cheap;
        }
        if other.model_routing.reasoning.is_some() {
            self.model_routing.reasoning = other.model_routing.reasoning;
        }
        if other.model_routing.auto_memory.is_some() {
            self.model_routing.auto_memory = other.model_routing.auto_memory;
        }

        // Sandbox: override if non-default
        if other.sandbox.preference != default_sandbox_preference() {
            self.sandbox.preference = other.sandbox.preference;
        }
        if other.sandbox.bubblewrap_path.is_some() {
            self.sandbox.bubblewrap_path = other.sandbox.bubblewrap_path;
        }

        // MCP servers: append
        self.mcp_servers.extend(other.mcp_servers);

        // Plugins: append directories
        self.plugins.directories.extend(other.plugins.directories);

        // Memory: override if non-default
        if other.memory.backend != default_memory_backend() {
            self.memory.backend = other.memory.backend;
        }
        if other.memory.data_dir != default_memory_data_dir() {
            self.memory.data_dir = other.memory.data_dir;
        }
        // gbrain: merge if enabled or non-default
        if other.memory.gbrain.enabled {
            self.memory.gbrain.enabled = true;
        }
        if other.memory.gbrain.server_name != default_gbrain_server_name() {
            self.memory.gbrain.server_name = other.memory.gbrain.server_name;
        }
        if other.memory.gbrain.source != default_gbrain_source() {
            self.memory.gbrain.source = other.memory.gbrain.source;
        }
        if other.memory.gbrain.general_source != default_general_source() {
            self.memory.gbrain.general_source = other.memory.gbrain.general_source;
        }
        if other.memory.gbrain.timeout_ms != default_gbrain_timeout_ms() {
            self.memory.gbrain.timeout_ms = other.memory.gbrain.timeout_ms;
        }
        if other.memory.gbrain.max_results != default_gbrain_max_results() {
            self.memory.gbrain.max_results = other.memory.gbrain.max_results;
        }
        if other.memory.gbrain.max_chars != default_gbrain_max_chars() {
            self.memory.gbrain.max_chars = other.memory.gbrain.max_chars;
        }
        if other.memory.gbrain.capture_enabled {
            self.memory.gbrain.capture_enabled = true;
        }
        if other.memory.gbrain.outbox_dir != default_gbrain_outbox_dir() {
            self.memory.gbrain.outbox_dir = other.memory.gbrain.outbox_dir;
        }

        // Daemon: override if non-default
        if other.daemon.socket_path != default_daemon_socket_path() {
            self.daemon.socket_path = other.daemon.socket_path;
        }
        if other.daemon.log_level != default_daemon_log_level() {
            self.daemon.log_level = other.daemon.log_level;
        }

        // Perception: override if non-default
        if other.perception.enabled {
            self.perception.enabled = other.perception.enabled;
        }
        if other.perception.watch_paths != default_perception_watch_paths() {
            self.perception.watch_paths = other.perception.watch_paths;
        }
        if !other.perception.enable_journald {
            self.perception.enable_journald = other.perception.enable_journald;
        }

        // Evolution: override if enabled downstream
        if other.evolution.enabled {
            self.evolution.enabled = other.evolution.enabled;
        }

        // Telegram: override if non-default
        if other.telegram.enabled {
            self.telegram.enabled = other.telegram.enabled;
        }
        if other.telegram.bot_token_env.is_some() {
            self.telegram.bot_token_env = other.telegram.bot_token_env;
        }
        if other.telegram.owner_user_id.is_some() {
            self.telegram.owner_user_id = other.telegram.owner_user_id;
        }
        if other.telegram.poll_timeout_secs != default_poll_timeout_secs() {
            self.telegram.poll_timeout_secs = other.telegram.poll_timeout_secs;
        }
    }

    /// Load config with layer merging (low → high precedence):
    /// - Layer 0: compiled defaults
    /// - Layer 1: /etc/aletheon/config.toml   (system defaults)
    /// - Layer 2: ~/.aletheon/config.toml     (user; authoritative for daily edits)
    /// - Layer 3: `<project>/.aletheon/config.toml` (project-local)
    pub fn load_layered(project_dir: Option<&Path>) -> Self {
        let mut config = Self::default();

        // Layer 1: system
        let etc_path = Path::new("/etc/aletheon/config.toml");
        if etc_path.exists() {
            if let Ok(sys_config) = Self::from_file(etc_path) {
                config.merge(sys_config);
            }
        }

        // Layer 2: user global
        let global_path = dirs::home_dir()
            .map(|h| h.join(".aletheon/config.toml"))
            .filter(|p| p.exists());
        if let Some(path) = global_path {
            if let Ok(user_config) = Self::from_file(&path) {
                config.merge(user_config);
            }
        }

        // Layer 3: project local
        if let Some(dir) = project_dir {
            let project_path = dir.join(".aletheon/config.toml");
            if project_path.exists() {
                if let Ok(project_config) = Self::from_file(&project_path) {
                    config.merge(project_config);
                }
            }
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_pricing_parses_and_defaults_to_none() {
        let with = r#"
            name = "anthropic"
            base_url = "https://api.anthropic.com"
            [pricing]
            input_per_1k = 3.0
            output_per_1k = 15.0
        "#;
        let p: ProviderConfig = toml::from_str(with).unwrap();
        let pr = p.pricing.expect("pricing present");
        assert_eq!(pr.input_per_1k, 3.0);
        assert_eq!(pr.output_per_1k, 15.0);

        let without = "name = \"local\"\nbase_url = \"http://localhost:11434\"\n";
        let p2: ProviderConfig = toml::from_str(without).unwrap();
        assert!(p2.pricing.is_none(), "pricing is optional");
    }

    #[test]
    fn merge_covers_perception_evolution_and_prompt() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.perception.enabled = true;
        other.agent.system_prompt = "OVERRIDDEN".into();
        other.agent.compaction_enabled = false;

        base.merge(other);

        assert!(base.perception.enabled, "perception must merge");
        assert_eq!(base.agent.system_prompt, "OVERRIDDEN");
        assert!(!base.agent.compaction_enabled);
    }

    #[test]
    fn merge_precedence_user_over_system() {
        // Unit-level proxy for layer precedence: later merge wins.
        let mut config = AppConfig::default();
        let mut system = AppConfig::default();
        system.agent.default_model = Some("system-model".into());
        let mut user = AppConfig::default();
        user.agent.default_model = Some("user-model".into());

        config.merge(system);
        config.merge(user);

        assert_eq!(config.agent.default_model.as_deref(), Some("user-model"));
    }

    // ── TelegramConfig validation ──────────────────────────────────────

    #[test]
    fn telegram_disabled_needs_no_token_or_owner() {
        let mut cfg = TelegramConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.bot_token_env.is_none());
        assert!(cfg.owner_user_id.is_none());
        let errors = cfg.validate();
        assert!(
            errors.is_empty(),
            "disabled config must have no errors, got: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_requires_token_env() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: None,
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject missing bot_token_env");
        assert!(
            errors.iter().any(|e| e.contains("bot_token_env")),
            "error must mention bot_token_env: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_rejects_empty_token_env() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("   ".to_string()),
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject empty bot_token_env");
        assert!(
            errors.iter().any(|e| e.contains("bot_token_env")),
            "error must mention bot_token_env: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_requires_owner_user_id() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
            owner_user_id: None,
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(!errors.is_empty(), "must reject missing owner_user_id");
        assert!(
            errors.iter().any(|e| e.contains("owner_user_id")),
            "error must mention owner_user_id: {errors:?}"
        );
    }

    #[test]
    fn telegram_enabled_rejects_zero_or_negative_owner_id() {
        for bad_id in [0, -1] {
            let mut cfg = TelegramConfig {
                enabled: true,
                bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
                owner_user_id: Some(bad_id),
                poll_timeout_secs: 10,
            };
            let errors = cfg.validate();
            assert!(!errors.is_empty(), "must reject owner_user_id={bad_id}");
            assert!(
                errors.iter().any(|e| e.contains("not positive")),
                "error must say 'not positive' for id={bad_id}: {errors:?}"
            );
        }
    }

    #[test]
    fn telegram_valid_enabled_passes() {
        let mut cfg = TelegramConfig {
            enabled: true,
            bot_token_env: Some("ALETHEON_TELEGRAM_BOT_TOKEN".into()),
            owner_user_id: Some(12345),
            poll_timeout_secs: 10,
        };
        let errors = cfg.validate();
        assert!(
            errors.is_empty(),
            "valid config must have no errors, got: {errors:?}"
        );
    }

    #[test]
    fn telegram_clamps_poll_timeout() {
        // Below minimum
        let mut cfg = TelegramConfig {
            enabled: false,
            bot_token_env: None,
            owner_user_id: None,
            poll_timeout_secs: 0,
        };
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 1);

        // Above maximum
        cfg.poll_timeout_secs = 100;
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 50);

        // In range
        cfg.poll_timeout_secs = 30;
        cfg.validate();
        assert_eq!(cfg.poll_timeout_secs, 30);
    }

    #[test]
    fn telegram_parses_from_toml() {
        let toml = r#"
enabled = true
bot_token_env = "ALETHEON_TELEGRAM_BOT_TOKEN"
owner_user_id = 12345
poll_timeout_secs = 20
"#;
        let cfg: TelegramConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(
            cfg.bot_token_env.as_deref(),
            Some("ALETHEON_TELEGRAM_BOT_TOKEN")
        );
        assert_eq!(cfg.owner_user_id, Some(12345));
        assert_eq!(cfg.poll_timeout_secs, 20);
    }

    #[test]
    fn telegram_default_in_app_config() {
        let toml = r#"
[[providers]]
name = "test"
base_url = "http://localhost"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert!(!config.telegram.enabled, "telegram disabled by default");
        assert_eq!(config.telegram.poll_timeout_secs, 10);
    }

    #[test]
    fn telegram_merge_overrides() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.telegram.enabled = true;
        other.telegram.bot_token_env = Some("MY_TOKEN".into());
        other.telegram.owner_user_id = Some(42);

        base.merge(other);

        assert!(base.telegram.enabled);
        assert_eq!(base.telegram.bot_token_env.as_deref(), Some("MY_TOKEN"));
        assert_eq!(base.telegram.owner_user_id, Some(42));
    }

    #[test]
    fn gbrain_disabled_by_default() {
        let config = AppConfig::default();
        assert!(!config.memory.gbrain.enabled);
        assert_eq!(config.memory.gbrain.server_name, "gbrain");
        assert_eq!(config.memory.gbrain.source, "aletheon");
        assert_eq!(config.memory.gbrain.timeout_ms, 1200);
        assert_eq!(config.memory.gbrain.max_results, 4);
        assert_eq!(config.memory.gbrain.max_chars, 6000);
        assert!(!config.memory.gbrain.capture_enabled);
    }

    #[test]
    fn gbrain_parses_from_toml() {
        let toml = r#"
enabled = true
server_name = "gbrain"
source = "aletheon"
general_source = "general"
timeout_ms = 1200
max_results = 4
max_chars = 6000
capture_enabled = false
outbox_dir = "~/.aletheon/gbrain-outbox"
"#;
        let cfg: GbrainMemoryConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.server_name, "gbrain");
        assert_eq!(cfg.source, "aletheon");
        assert_eq!(cfg.timeout_ms, 1200);
        assert!(!cfg.capture_enabled);
    }

    #[test]
    fn gbrain_nested_in_memory_config() {
        let toml = r#"
backend = "sqlite"
data_dir = "/tmp/mem"

[gbrain]
enabled = true
source = "aletheon"
timeout_ms = 2500
max_results = 6
"#;
        let mem: MemoryConfig = toml::from_str(toml).unwrap();
        assert!(mem.gbrain.enabled);
        assert_eq!(mem.gbrain.source, "aletheon");
        assert_eq!(mem.gbrain.timeout_ms, 2500);
        assert_eq!(mem.gbrain.server_name, "gbrain");
    }

    #[test]
    fn gbrain_merge_overrides() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.memory.gbrain.enabled = true;
        other.memory.gbrain.source = "custom".into();
        other.memory.gbrain.timeout_ms = 5000;

        base.merge(other);

        assert!(base.memory.gbrain.enabled);
        assert_eq!(base.memory.gbrain.source, "custom");
        assert_eq!(base.memory.gbrain.timeout_ms, 5000);
    }
}
