//! Configuration types shared between brain-core and runtime.
//!
//! These types were originally in the core crate, then moved to aletheon-runtime.
//! Duplicated here to break the cyclic dependency (brain-core <-> runtime).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Dynamic model routing — maps task types to model specs.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            default: None,
            multimodal: None,
            cheap: None,
            reasoning: None,
            auto_memory: None,
        }
    }
}

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Openai,
    Anthropic,
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

        // Daemon: override if non-default
        if other.daemon.socket_path != default_daemon_socket_path() {
            self.daemon.socket_path = other.daemon.socket_path;
        }
        if other.daemon.log_level != default_daemon_log_level() {
            self.daemon.log_level = other.daemon.log_level;
        }
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
            model_aliases: HashMap::new(),
            model_routing: ModelRoutingConfig::default(),
            sandbox: SandboxConfig::default(),
            mcp_servers: Vec::new(),
            plugins: PluginsConfig::default(),
            memory: MemoryConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}
