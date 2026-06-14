use serde::{Deserialize, Serialize};

/// Type of inference provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderType {
    Local,
    Cloud,
}

/// Configuration for an inference provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider_type: ProviderType,
    pub model: String,
    pub api_url: Option<String>,
    pub max_context_length: usize,
    pub cost_per_1k_tokens: f64,
    pub latency_ms: u64,
}
