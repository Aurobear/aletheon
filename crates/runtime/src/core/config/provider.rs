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
}
