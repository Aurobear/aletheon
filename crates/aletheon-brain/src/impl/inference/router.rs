use tracing::debug;
use super::InferenceConfig;
use super::classifier::{IntentClassifier, Complexity};
use super::provider_config::{ProviderConfig, ProviderType};

/// Routes inference requests to appropriate providers.
pub struct InferenceRouter {
    providers: Vec<ProviderConfig>,
    classifier: IntentClassifier,
    /// Model identifier for local provider (from InferenceConfig).
    local_model: String,
    /// Model identifier for cloud provider (from InferenceConfig).
    cloud_model: String,
    /// Iteration threshold for runtime upgrade.
    upgrade_threshold_iterations: usize,
    /// Tool-call threshold for runtime upgrade.
    upgrade_threshold_tool_calls: usize,
}

impl InferenceRouter {
    pub fn new(providers: Vec<ProviderConfig>) -> Self {
        Self {
            providers,
            classifier: IntentClassifier::new(),
            local_model: "local".into(),
            cloud_model: "cloud".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        }
    }

    /// Create an InferenceRouter from an InferenceConfig, building default providers.
    pub fn from_config(config: &InferenceConfig) -> Self {
        let providers = vec![
            ProviderConfig {
                id: "local".into(),
                name: config.local_model.clone(),
                provider_type: ProviderType::Local,
                model: config.local_model.clone(),
                api_url: None,
                max_context_length: 8192,
                cost_per_1k_tokens: 0.0,
                latency_ms: 100,
            },
            ProviderConfig {
                id: "cloud".into(),
                name: config.cloud_model.clone(),
                provider_type: ProviderType::Cloud,
                model: config.cloud_model.clone(),
                api_url: None,
                max_context_length: 200_000,
                cost_per_1k_tokens: 0.01,
                latency_ms: 800,
            },
        ];
        Self {
            providers,
            classifier: IntentClassifier::new(),
            local_model: config.local_model.clone(),
            cloud_model: config.cloud_model.clone(),
            upgrade_threshold_iterations: config.upgrade_threshold_iterations,
            upgrade_threshold_tool_calls: config.upgrade_threshold_tool_calls,
        }
    }

    /// Select the best provider for a given message.
    pub fn select_provider(&self, message: &str) -> &ProviderConfig {
        let complexity = self.classifier.classify(message);
        debug!(?complexity, "Classified message complexity");

        match complexity {
            Complexity::Simple => {
                // Use local provider if available
                self.providers.iter()
                    .filter(|p| matches!(p.provider_type, ProviderType::Local))
                    .min_by_key(|p| p.latency_ms)
                    .or_else(|| self.providers.first())
                    .unwrap()
            }
            Complexity::Medium => {
                // Use cost-optimal cloud provider
                self.providers.iter()
                    .filter(|p| matches!(p.provider_type, ProviderType::Cloud))
                    .min_by(|a, b| a.cost_per_1k_tokens.partial_cmp(&b.cost_per_1k_tokens).unwrap())
                    .or_else(|| self.providers.first())
                    .unwrap()
            }
            Complexity::Complex => {
                // Use quality-first cloud provider (longest context)
                self.providers.iter()
                    .filter(|p| matches!(p.provider_type, ProviderType::Cloud))
                    .max_by_key(|p| p.max_context_length)
                    .or_else(|| self.providers.first())
                    .unwrap()
            }
        }
    }

    /// Select provider by model identifier for a given message.
    /// Returns the model string that should be used for the LLM call.
    pub fn select_model(&self, message: &str) -> &str {
        let provider = self.select_provider(message);
        &provider.model
    }

    /// Get the local model identifier.
    pub fn local_model(&self) -> &str {
        &self.local_model
    }

    /// Get the cloud model identifier.
    pub fn cloud_model(&self) -> &str {
        &self.cloud_model
    }

    /// Check if should upgrade to cloud based on runtime metrics.
    /// Uses configurable thresholds.
    pub fn should_upgrade(&self, iterations: usize, tool_calls: usize) -> bool {
        iterations > self.upgrade_threshold_iterations
            || tool_calls > self.upgrade_threshold_tool_calls
    }

    /// Return the model to use, considering both message complexity and runtime metrics.
    /// This is the main entry point for the engine: pass in the user message plus
    /// current iteration/tool-call counts, and get back which model to call.
    pub fn route(&self, message: &str, iterations: usize, total_tool_calls: usize) -> &str {
        if self.should_upgrade(iterations, total_tool_calls) {
            debug!(
                iterations,
                total_tool_calls,
                "Runtime upgrade threshold exceeded, forcing cloud model"
            );
            return &self.cloud_model;
        }
        self.select_model(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_providers() -> Vec<ProviderConfig> {
        vec![
            ProviderConfig {
                id: "local".into(),
                name: "Local LLM".into(),
                provider_type: ProviderType::Local,
                model: "qwen3-8b".into(),
                api_url: None,
                max_context_length: 8192,
                cost_per_1k_tokens: 0.0,
                latency_ms: 100,
            },
            ProviderConfig {
                id: "cloud-cheap".into(),
                name: "Cloud Cheap".into(),
                provider_type: ProviderType::Cloud,
                model: "deepseek-v3".into(),
                api_url: Some("https://api.deepseek.com".into()),
                max_context_length: 32768,
                cost_per_1k_tokens: 0.001,
                latency_ms: 500,
            },
            ProviderConfig {
                id: "cloud-quality".into(),
                name: "Cloud Quality".into(),
                provider_type: ProviderType::Cloud,
                model: "claude-opus".into(),
                api_url: Some("https://api.anthropic.com".into()),
                max_context_length: 200000,
                cost_per_1k_tokens: 0.015,
                latency_ms: 1000,
            },
        ]
    }

    #[test]
    fn test_simple_routes_to_local() {
        let router = InferenceRouter::new(make_providers());
        let provider = router.select_provider("read file /etc/hostname");
        assert_eq!(provider.id, "local");
    }

    #[test]
    fn test_complex_routes_to_quality() {
        let router = InferenceRouter::new(make_providers());
        let provider = router.select_provider("analyze the architecture and design a migration plan");
        assert_eq!(provider.id, "cloud-quality");
    }

    #[test]
    fn test_upgrade_threshold() {
        let router = InferenceRouter::new(make_providers());
        assert!(!router.should_upgrade(3, 5));
        assert!(router.should_upgrade(6, 5));
        assert!(router.should_upgrade(3, 9));
    }

    #[test]
    fn test_from_config_creates_providers() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        };
        let router = InferenceRouter::from_config(&config);
        assert_eq!(router.local_model(), "llama-3.2-1b");
        assert_eq!(router.cloud_model(), "claude-sonnet-4-20250514");
        assert_eq!(router.providers.len(), 2);
    }

    #[test]
    fn test_route_simple_message_below_threshold() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        };
        let router = InferenceRouter::from_config(&config);
        // Simple message, below thresholds -> should route to local
        let model = router.route("read file /etc/hostname", 0, 0);
        assert_eq!(model, "llama-3.2-1b");
    }

    #[test]
    fn test_route_complex_message_below_threshold() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        };
        let router = InferenceRouter::from_config(&config);
        // Complex message, below thresholds -> should route to cloud (quality)
        let model = router.route("analyze the architecture and design a migration plan", 0, 0);
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_route_runtime_upgrade_by_iterations() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        };
        let router = InferenceRouter::from_config(&config);
        // Simple message but iteration > 5 -> should force cloud
        let model = router.route("read file /etc/hostname", 6, 0);
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_route_runtime_upgrade_by_tool_calls() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 5,
            upgrade_threshold_tool_calls: 8,
        };
        let router = InferenceRouter::from_config(&config);
        // Simple message but tool_calls > 8 -> should force cloud
        let model = router.route("read file /etc/hostname", 0, 9);
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_custom_thresholds() {
        let config = InferenceConfig {
            enabled: true,
            local_model: "llama-3.2-1b".into(),
            cloud_model: "claude-sonnet-4-20250514".into(),
            upgrade_threshold_iterations: 3,
            upgrade_threshold_tool_calls: 5,
        };
        let router = InferenceRouter::from_config(&config);
        // iteration=4 > threshold=3 -> upgrade
        assert!(router.should_upgrade(4, 0));
        // tool_calls=6 > threshold=5 -> upgrade
        assert!(router.should_upgrade(0, 6));
        // both below -> no upgrade
        assert!(!router.should_upgrade(2, 4));
    }
}
