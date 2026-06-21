//! LLM provider configuration: Transport, ProviderConfig, ModelRoutingConfig.

use serde::{Deserialize, Serialize};

/// Re-export ModelRoutingConfig from aletheon-brain to avoid duplicate types.
pub use cognit::config::ModelRoutingConfig;

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
    /// Override the default max context length for this provider's models.
    /// If not set, the provider uses its built-in default (128K for OpenAI, 200K for Anthropic).
    /// Use `model_context_limits` in config.toml for per-model overrides.
    #[serde(default)]
    pub max_context_length: Option<usize>,
}
