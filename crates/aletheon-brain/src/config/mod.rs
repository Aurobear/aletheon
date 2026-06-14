//! Configuration types shared between brain-core and runtime.
//!
//! These types were originally in the core crate, then moved to aletheon-runtime.
//! Duplicated here to break the cyclic dependency (brain-core <-> runtime).

use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

/// Top-level application config (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
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
    Openai,
    Anthropic,
    Auto,
}

impl Default for Transport {
    fn default() -> Self { Self::Auto }
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
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::from_file(path).unwrap_or_default()
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            providers: Vec::new(),
            model_aliases: HashMap::new(),
        }
    }
}
