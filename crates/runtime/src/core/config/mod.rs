//! Application and runtime configuration.

mod agent;
mod genome;
mod infra;
mod provider;

pub use agent::{
    AgentConfig, AgentLoopConfig, CircuitBreakerConfig, EvolutionSettings, HooksConfig,
    PerceptionConfig, RuntimeConfig,
};
pub use genome::GenomeConfig;
pub use infra::{DaemonConfig, McpServerConfig, MemoryConfig, PluginsConfig, SandboxConfig};
pub use provider::{ModelRoutingConfig, ProviderConfig, Transport};

use agent::{
    default_compaction_keep_recent, default_compaction_threshold, default_max_iterations,
    default_max_tokens,
};
use infra::{
    default_daemon_log_level, default_daemon_socket_path, default_memory_backend,
    default_memory_data_dir, default_sandbox_preference,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub evolution: EvolutionSettings,
}

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
    fn shipped_default_config_is_startable_shaped() {
        // repo-root config/default.toml relative to this crate (crates/runtime)
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/default.toml");
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let cfg: AppConfig = toml::from_str(&text).expect("default.toml must parse");
        assert!(
            !cfg.providers.is_empty(),
            "default.toml must define >=1 provider"
        );
        let dp = cfg
            .agent
            .default_provider
            .as_deref()
            .expect("default.toml must set agent.default_provider");
        assert!(
            cfg.providers.iter().any(|p| p.name == dp),
            "default_provider '{dp}' must match a [[providers]] name"
        );
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
socket_path = "/run/aletheond/aletheond.sock"
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
            max_context_length: None,
        });

        let mut other = AppConfig::default();
        other.providers.push(ProviderConfig {
            name: "openai".to_string(),
            base_url: "https://api.openai.com/v2".to_string(),
            api_key: "sk-new".to_string(),
            transport: Transport::Openai,
            models: vec!["gpt-4o".to_string()],
            max_context_length: None,
        });
        other.providers.push(ProviderConfig {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: String::new(),
            transport: Transport::Anthropic,
            models: vec![],
            max_context_length: None,
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

        base.merge(other);

        assert_eq!(base.model_routing.default.as_deref(), Some("other/default"));
        assert_eq!(
            base.model_routing.multimodal.as_deref(),
            Some("other/multimodal")
        );
        assert_eq!(base.model_routing.cheap.as_deref(), Some("base/cheap"));
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
        assert_eq!(base.daemon.socket_path, "/run/aletheond/aletheond.sock");
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
        assert_eq!(config.hooks.post_tool, vec!["/usr/local/bin/post_tool.sh"]);
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
