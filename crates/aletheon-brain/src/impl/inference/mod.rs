pub mod classifier;
pub mod provider_config;
pub mod router;

pub use classifier::IntentClassifier;
pub use provider_config::{ProviderConfig, ProviderType};
pub use router::InferenceRouter;

use serde::{Deserialize, Serialize};

/// Top-level configuration for the inference routing subsystem.
///
/// Deserialized from TOML `[inference]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Whether inference routing is enabled.
    pub enabled: bool,
    /// Model identifier for the local (cheap/fast) provider.
    pub local_model: String,
    /// Model identifier for the cloud (quality) provider.
    pub cloud_model: String,
    /// Number of iterations before auto-upgrading to cloud.
    #[serde(default = "default_upgrade_threshold_iterations")]
    pub upgrade_threshold_iterations: usize,
    /// Number of tool calls before auto-upgrading to cloud.
    #[serde(default = "default_upgrade_threshold_tool_calls")]
    pub upgrade_threshold_tool_calls: usize,
}

fn default_upgrade_threshold_iterations() -> usize {
    5
}

fn default_upgrade_threshold_tool_calls() -> usize {
    8
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            local_model: "llama-3.2-1b".to_string(),
            cloud_model: "claude-sonnet-4-20250514".to_string(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        }
    }
}
