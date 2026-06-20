use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig — top-level application config
// ---------------------------------------------------------------------------

/// Re-export ModelRoutingConfig from aletheon-brain to avoid duplicate types.
pub use cognit::config::ModelRoutingConfig;

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_aliases: std::collections::HashMap<String, String>,
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
    pub hooks: HooksConfig,
    #[serde(default)]
    pub perception: PerceptionConfig,
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

fn default_max_iterations() -> usize {
    25
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
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user.".to_string()
}

/// Wire protocol between client and LLM server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// OpenAI chat/completions API (also covers Ollama, LM Studio, vLLM, DeepSeek, etc.)
    Openai,
    /// Anthropic messages API (native)
    Anthropic,
    /// Auto-detect from base_url
    Auto,
}

impl Default for Transport {
    fn default() -> Self {
        Self::Auto
    }
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
        }
    }
}

/// Plugin directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default)]
    pub directories: Vec<String>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            directories: Vec::new(),
        }
    }
}

/// Memory backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// "sqlite" or "in_memory"
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
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
    "/run/aletheon/aletheon.sock".to_string()
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

/// Hook script configuration.
///
/// Each field is a list of script paths to execute at the specified lifecycle point.
/// Paths may use `~` for home directory expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            pre_turn: Vec::new(),
            post_tool: Vec::new(),
            on_session_end: Vec::new(),
            pre_tool: Vec::new(),
        }
    }
}

/// Perception subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionConfig {
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
            watch_paths: default_perception_watch_paths(),
            enable_journald: true,
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig methods
// ---------------------------------------------------------------------------

impl AppConfig {
    /// Load config from a TOML file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load from file if it exists, otherwise return default.
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
        // compaction fields use defaults — only override if explicitly set
        // (serde defaults are the same as Default, so we override if different from defaults)
        if other.agent.compaction_keep_recent != default_compaction_keep_recent() {
            self.agent.compaction_keep_recent = other.agent.compaction_keep_recent;
        }
        if other.agent.compaction_threshold != default_compaction_threshold() {
            self.agent.compaction_threshold = other.agent.compaction_threshold;
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

        // MCP servers: append (name dedup not enforced — user controls)
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

        // Daemon: override if non-default
        if other.daemon.socket_path != default_daemon_socket_path() {
            self.daemon.socket_path = other.daemon.socket_path;
        }
        if other.daemon.log_level != default_daemon_log_level() {
            self.daemon.log_level = other.daemon.log_level;
        }

        // Hooks: append script paths (allow multiple config layers to add hooks)
        self.hooks.pre_turn.extend(other.hooks.pre_turn);
        self.hooks.post_tool.extend(other.hooks.post_tool);
        self.hooks.on_session_end.extend(other.hooks.on_session_end);
        self.hooks.pre_tool.extend(other.hooks.pre_tool);
    }

    /// Load config with layer merging:
    /// - Layer 0: compiled defaults
    /// - Layer 1: user global (~/.aletheon/config.toml)
    /// - Layer 2: project local (.aletheon/config.toml in `project_dir`)
    pub fn load_layered(project_dir: Option<&Path>) -> Self {
        let mut config = Self::default();

        // Layer 1: user global
        let global_path = dirs::home_dir()
            .map(|h| h.join(".aletheon/config.toml"))
            .filter(|p| p.exists());
        if let Some(path) = global_path {
            if let Ok(user_config) = Self::from_file(&path) {
                config.merge(user_config);
            }
        }

        // Layer 2: project local
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            providers: Vec::new(),
            model_aliases: std::collections::HashMap::new(),
            model_routing: ModelRoutingConfig::default(),
            sandbox: SandboxConfig::default(),
            mcp_servers: Vec::new(),
            plugins: PluginsConfig::default(),
            memory: MemoryConfig::default(),
            daemon: DaemonConfig::default(),
            hooks: HooksConfig::default(),
            perception: PerceptionConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// GenomeConfig — genome-derived behavior parameters
// ---------------------------------------------------------------------------

/// Lightweight genome config snapshot held by the runtime.
/// Extracted from GenomeMeta — does not hold the full genome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeConfig {
    /// Reasoning strategy name (e.g., "plan-then-execute", "react").
    pub reasoning_strategy: String,
    /// Confidence threshold below which the agent considers itself stuck.
    pub impasse_threshold: f64,
    /// What triggers reflection.
    pub reflection_trigger: String,
    /// Care weights by topic (e.g., "safety" -> 1.0).
    pub care_weights: HashMap<String, f64>,
    /// Current genome version string.
    pub genome_version: String,
}

impl Default for GenomeConfig {
    fn default() -> Self {
        Self {
            reasoning_strategy: "plan-then-execute".to_string(),
            impasse_threshold: 0.3,
            reflection_trigger: "task_complete".to_string(),
            care_weights: HashMap::new(),
            genome_version: "0.1.0".to_string(),
        }
    }
}

impl GenomeConfig {
    /// Extract from a GenomeMeta.
    pub fn from_genome_meta(meta: &metacog::GenomeMeta) -> Self {
        Self {
            reasoning_strategy: meta.reasoning.default_strategy.clone(),
            impasse_threshold: meta.reasoning.impasse_threshold,
            reflection_trigger: meta.reasoning.reflection_trigger.clone(),
            care_weights: meta.care_ext.weights.clone(),
            genome_version: meta.genome_version.clone(),
        }
    }

    /// Format care weights for injection into system prompt.
    pub fn care_weights_prompt(&self) -> String {
        if self.care_weights.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = self.care_weights.iter()
            .map(|(k, v)| format!("  {}: {:.2}", k, v))
            .collect();
        parts.sort();
        format!("Current care priorities:\n{}", parts.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[agent]
default_provider = "mimo"
default_model = "mimo-v2.5-pro"

[[providers]]
name = "mimo"
base_url = "https://api.example.com"
api_key = "sk-test"
transport = "auto"
models = ["mimo-v2.5-pro"]
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.agent.default_provider.as_deref(), Some("mimo"));
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].name, "mimo");
    }

    #[test]
    fn test_parse_with_aliases() {
        let toml = r#"
[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
transport = "anthropic"

[model_aliases]
sonnet = "anthropic/claude-sonnet-4-20250514"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.model_aliases["sonnet"],
            "anthropic/claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.agent.max_iterations, 25);
        assert!(config.providers.is_empty());
        assert_eq!(config.sandbox.preference, "auto");
        assert_eq!(config.memory.backend, "sqlite");
        assert_eq!(config.daemon.log_level, "info");
    }

    #[test]
    fn test_runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.max_iterations, 50);
        assert!(config.compaction_enabled);
    }

    #[test]
    fn test_parse_full_config_with_new_sections() {
        let toml = r#"
[agent]
default_provider = "mimo"
default_model = "mimo-v2.5-pro"

[sandbox]
preference = "require"
bubblewrap_path = "/usr/bin/bwrap"

[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "mcp-fs"

[[mcp_servers]]
name = "web"
transport = "http"
url = "http://localhost:8080"

[plugins]
directories = ["/opt/aletheon/plugins"]

[memory]
backend = "sqlite"
data_dir = "/var/lib/aletheon/memory"

[daemon]
socket_path = "/run/aletheon/aletheon.sock"
log_level = "debug"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.sandbox.preference, "require");
        assert_eq!(
            config.sandbox.bubblewrap_path.as_deref(),
            Some("/usr/bin/bwrap")
        );
        assert_eq!(config.mcp_servers.len(), 2);
        assert_eq!(config.mcp_servers[0].name, "filesystem");
        assert_eq!(config.mcp_servers[1].transport, "http");
        assert_eq!(config.plugins.directories, vec!["/opt/aletheon/plugins"]);
        assert_eq!(config.memory.backend, "sqlite");
        assert_eq!(config.daemon.log_level, "debug");
    }

    #[test]
    fn test_merge_agent_overrides() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.agent.default_provider = Some("anthropic".to_string());
        other.agent.default_model = Some("claude-sonnet-4-20250514".to_string());

        base.merge(other);

        assert_eq!(base.agent.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(
            base.agent.default_model.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
    }

    #[test]
    fn test_merge_providers_by_name() {
        let mut base = AppConfig::default();
        base.providers.push(ProviderConfig {
            name: "openai".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_key: String::new(),
            transport: Transport::Openai,
            models: vec!["gpt-4".to_string()],
        });

        let mut other = AppConfig::default();
        // Same name — should replace
        other.providers.push(ProviderConfig {
            name: "openai".to_string(),
            base_url: "https://api.openai.com/v2".to_string(),
            api_key: "sk-new".to_string(),
            transport: Transport::Openai,
            models: vec!["gpt-4o".to_string()],
        });
        // New provider — should append
        other.providers.push(ProviderConfig {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: String::new(),
            transport: Transport::Anthropic,
            models: vec![],
        });

        base.merge(other);

        assert_eq!(base.providers.len(), 2);
        assert_eq!(base.providers[0].base_url, "https://api.openai.com/v2");
        assert_eq!(base.providers[1].name, "anthropic");
    }

    #[test]
    fn test_merge_model_aliases() {
        let mut base = AppConfig::default();
        base.model_aliases
            .insert("fast".to_string(), "gpt-3.5-turbo".to_string());

        let mut other = AppConfig::default();
        other
            .model_aliases
            .insert("fast".to_string(), "gpt-4o-mini".to_string());
        other
            .model_aliases
            .insert("smart".to_string(), "gpt-4o".to_string());

        base.merge(other);

        assert_eq!(base.model_aliases["fast"], "gpt-4o-mini");
        assert_eq!(base.model_aliases["smart"], "gpt-4o");
    }

    #[test]
    fn test_merge_model_routing() {
        let mut base = AppConfig::default();
        base.model_routing.default = Some("base/default".to_string());
        base.model_routing.cheap = Some("base/cheap".to_string());

        let mut other = AppConfig::default();
        other.model_routing.default = Some("other/default".to_string());
        other.model_routing.multimodal = Some("other/multimodal".to_string());
        other.model_routing.reasoning = Some("other/reasoning".to_string());
        // cheap not set in other — should keep base value

        base.merge(other);

        assert_eq!(
            base.model_routing.default.as_deref(),
            Some("other/default")
        );
        assert_eq!(
            base.model_routing.multimodal.as_deref(),
            Some("other/multimodal")
        );
        assert_eq!(
            base.model_routing.cheap.as_deref(),
            Some("base/cheap")
        );
        assert_eq!(
            base.model_routing.reasoning.as_deref(),
            Some("other/reasoning")
        );
        assert!(base.model_routing.auto_memory.is_none());
    }

    #[test]
    fn test_merge_sandbox_and_daemon() {
        let mut base = AppConfig::default();
        let mut other = AppConfig::default();
        other.sandbox.preference = "require".to_string();
        other.daemon.log_level = "debug".to_string();

        base.merge(other);

        assert_eq!(base.sandbox.preference, "require");
        assert_eq!(base.daemon.log_level, "debug");
        // Default values should not override
        assert_eq!(base.daemon.socket_path, "/run/aletheon/aletheon.sock");
    }

    #[test]
    fn test_merge_mcp_servers_append() {
        let mut base = AppConfig::default();
        base.mcp_servers.push(McpServerConfig {
            name: "fs".to_string(),
            transport: "stdio".to_string(),
            command: Some("mcp-fs".to_string()),
            url: None,
        });

        let mut other = AppConfig::default();
        other.mcp_servers.push(McpServerConfig {
            name: "web".to_string(),
            transport: "http".to_string(),
            command: None,
            url: Some("http://localhost:8080".to_string()),
        });

        base.merge(other);

        assert_eq!(base.mcp_servers.len(), 2);
        assert_eq!(base.mcp_servers[0].name, "fs");
        assert_eq!(base.mcp_servers[1].name, "web");
    }

    #[test]
    fn test_merge_plugins_directories_append() {
        let mut base = AppConfig::default();
        base.plugins.directories.push("/base/plugins".to_string());

        let mut other = AppConfig::default();
        other.plugins.directories.push("/other/plugins".to_string());

        base.merge(other);

        assert_eq!(base.plugins.directories.len(), 2);
        assert_eq!(base.plugins.directories[0], "/base/plugins");
        assert_eq!(base.plugins.directories[1], "/other/plugins");
    }

    #[test]
    fn test_load_layered_global_only() {
        let config = AppConfig::load_layered(None);
        // Should return defaults (global may or may not exist)
        assert_eq!(config.agent.max_iterations, 25);
    }

    #[test]
    fn test_load_layered_project_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path();
        let aletheon_dir = project_dir.join(".aletheon");
        std::fs::create_dir_all(&aletheon_dir).unwrap();

        let project_config = r#"
[agent]
default_provider = "project-provider"

[sandbox]
preference = "require"
"#;
        std::fs::write(aletheon_dir.join("config.toml"), project_config).unwrap();

        let config = AppConfig::load_layered(Some(project_dir));
        assert_eq!(
            config.agent.default_provider.as_deref(),
            Some("project-provider")
        );
        assert_eq!(config.sandbox.preference, "require");
    }

    #[test]
    fn test_load_layered_no_project_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config = AppConfig::load_layered(Some(tmp.path()));
        // Should still be defaults since no .aletheon/config.toml exists
        assert_eq!(config.agent.max_iterations, 25);
    }

    #[test]
    fn config_has_compaction_defaults() {
        let config = RuntimeConfig::default();
        assert_eq!(config.tail_token_budget, 16_000);
        assert_eq!(config.target_summary_chars, 2_000);
        assert_eq!(config.context_window_tokens, 128_000);
        assert!(config.compaction_enabled);
    }

    #[test]
    fn test_hooks_config_default() {
        let hooks = HooksConfig::default();
        assert!(hooks.pre_turn.is_empty());
        assert!(hooks.post_tool.is_empty());
        assert!(hooks.on_session_end.is_empty());
        assert!(hooks.pre_tool.is_empty());
    }

    #[test]
    fn test_hooks_config_from_toml() {
        let toml = r#"
[hooks]
pre_turn = ["~/.aletheon/hooks/pre_turn.sh"]
post_tool = ["/usr/local/bin/post_tool.sh"]
on_session_end = ["~/.aletheon/hooks/cleanup.sh"]
pre_tool = []

[[providers]]
name = "test"
base_url = "http://localhost"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.hooks.pre_turn, vec!["~/.aletheon/hooks/pre_turn.sh"]);
        assert_eq!(
            config.hooks.post_tool,
            vec!["/usr/local/bin/post_tool.sh"]
        );
        assert_eq!(
            config.hooks.on_session_end,
            vec!["~/.aletheon/hooks/cleanup.sh"]
        );
        assert!(config.hooks.pre_tool.is_empty());
    }

    #[test]
    fn test_hooks_config_default_in_app_config() {
        let toml = r#"
[[providers]]
name = "test"
base_url = "http://localhost"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        // hooks section absent => defaults to empty
        assert!(config.hooks.pre_turn.is_empty());
        assert!(config.hooks.post_tool.is_empty());
        assert!(config.hooks.on_session_end.is_empty());
        assert!(config.hooks.pre_tool.is_empty());
    }

    #[test]
    fn test_merge_hooks_append() {
        let mut base = AppConfig::default();
        base.hooks.pre_turn.push("/base/pre.sh".to_string());

        let mut other = AppConfig::default();
        other.hooks.pre_turn.push("/other/pre.sh".to_string());
        other.hooks.on_session_end.push("/other/end.sh".to_string());

        base.merge(other);

        assert_eq!(base.hooks.pre_turn.len(), 2);
        assert_eq!(base.hooks.pre_turn[0], "/base/pre.sh");
        assert_eq!(base.hooks.pre_turn[1], "/other/pre.sh");
        assert_eq!(base.hooks.on_session_end, vec!["/other/end.sh"]);
    }

    // ---- GenomeConfig tests ----

    #[test]
    fn test_care_weights_prompt_empty() {
        let config = GenomeConfig::default();
        assert_eq!(config.care_weights_prompt(), "");
    }

    #[test]
    fn test_care_weights_prompt_with_values() {
        let mut config = GenomeConfig::default();
        config.care_weights.insert("safety".to_string(), 1.0);
        config.care_weights.insert("helpfulness".to_string(), 0.8);
        let prompt = config.care_weights_prompt();
        assert!(prompt.contains("safety: 1.00"));
        assert!(prompt.contains("helpfulness: 0.80"));
    }

    #[test]
    fn test_genome_config_default_values() {
        let config = GenomeConfig::default();
        assert_eq!(config.reasoning_strategy, "plan-then-execute");
        assert_eq!(config.impasse_threshold, 0.3);
        assert_eq!(config.genome_version, "0.1.0");
    }

    #[test]
    fn test_perception_config_default() {
        let config = PerceptionConfig::default();
        assert_eq!(config.watch_paths, vec!["/etc", "/var/log"]);
        assert!(config.enable_journald);
    }

    #[test]
    fn test_perception_config_from_toml() {
        let toml = r#"
[perception]
watch_paths = ["/tmp", "/home/user/logs"]
enable_journald = false
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.perception.watch_paths,
            vec!["/tmp", "/home/user/logs"]
        );
        assert!(!config.perception.enable_journald);
    }

    #[test]
    fn test_perception_config_default_in_app_config() {
        let toml = r#"
[[providers]]
name = "test"
base_url = "http://localhost"
"#;
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.perception.watch_paths, vec!["/etc", "/var/log"]);
        assert!(config.perception.enable_journald);
    }
}
