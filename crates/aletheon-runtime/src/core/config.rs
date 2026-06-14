use std::path::Path;
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
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            session_id: uuid::Uuid::new_v4().to_string(),
            learning_enabled: true,
            compaction_enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig — merged from argos-core/src/config.rs
// ---------------------------------------------------------------------------

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_aliases: std::collections::HashMap<String, String>,
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
        }
    }
}

fn default_max_iterations() -> usize { 25 }
fn default_max_tokens() -> usize { 100_000 }
fn default_true() -> bool { true }
fn default_compaction_keep_recent() -> usize { 10 }
fn default_compaction_threshold() -> usize { 30 }

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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            providers: Vec::new(),
            model_aliases: std::collections::HashMap::new(),
        }
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
        assert_eq!(config.model_aliases["sonnet"], "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.agent.max_iterations, 25);
        assert!(config.providers.is_empty());
    }

    #[test]
    fn test_runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.max_iterations, 50);
        assert!(config.compaction_enabled);
    }
}
