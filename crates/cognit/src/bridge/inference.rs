use crate::r#impl::inference::{InferenceConfig, InferenceRouter, ProviderConfig};

/// Bridges InferenceRouter into CognitCore.
///
/// Routes inference requests to the optimal provider based on message complexity
/// and runtime upgrade thresholds (iteration count, tool-call count).
pub struct InferenceBridge {
    router: InferenceRouter,
}

impl InferenceBridge {
    /// Create a bridge from an InferenceConfig.
    /// Internally builds default Local + Cloud ProviderConfigs.
    pub fn new(config: InferenceConfig) -> Self {
        Self {
            router: InferenceRouter::from_config(&config),
        }
    }

    /// Select the optimal provider for a given message text.
    pub fn select_provider(&self, message: &str) -> &ProviderConfig {
        self.router.select_provider(message)
    }

    /// Select the model identifier for a given message text.
    pub fn select_model(&self, message: &str) -> &str {
        self.router.select_model(message)
    }

    /// Check if we should upgrade to a more capable provider based on runtime metrics.
    pub fn should_upgrade(&self, iterations: usize, tool_calls: usize) -> bool {
        self.router.should_upgrade(iterations, tool_calls)
    }

    /// Route to the best model, considering both message complexity and runtime metrics.
    /// This is the main entry point for CognitCore callers.
    pub fn route(&self, message: &str, iterations: usize, total_tool_calls: usize) -> &str {
        self.router.route(message, iterations, total_tool_calls)
    }
}
