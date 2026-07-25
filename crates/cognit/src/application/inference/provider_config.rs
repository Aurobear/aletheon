use serde::{Deserialize, Serialize};

/// Scheduling class for an inference candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderClass {
    Local,
    Cloud,
}

/// A model candidate used only by the heuristic inference router.
///
/// This is deliberately not the canonical provider configuration. Transport,
/// credentials, timeouts and pricing are owned by `crate::config::ProviderConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceCandidate {
    pub id: String,
    pub name: String,
    pub provider_class: ProviderClass,
    pub model: String,
    pub api_url: Option<String>,
    pub max_context_length: usize,
    pub cost_per_1k_tokens: f64,
    pub latency_ms: u64,
}
