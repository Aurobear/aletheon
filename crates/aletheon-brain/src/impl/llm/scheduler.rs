//! Centralized LLM routing.
//!
//! Other modules do NOT hold LlmProvider directly. They call LlmScheduler::request()
//! which routes to the right provider based on LlmPurpose.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use aletheon_abi::evolution::{LlmPurpose, ProviderHealth};
use aletheon_abi::message::Message;

use crate::config::{ProviderConfig, Transport};
use super::provider::{LlmProvider, LlmResponse, ToolDefinition};
use super::provider_factory::create_provider_by_kind;

/// Routing rule: maps a purpose to a provider name.
#[derive(Debug, Clone)]
pub struct RoutingRule {
    pub purpose: LlmPurpose,
    pub provider_name: String,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone)]
pub struct SchedulerProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub kind: String,  // "anthropic" | "openai" | "ollama"
    pub model: String,
}

/// Full scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub providers: Vec<SchedulerProviderConfig>,
    pub routing: Vec<RoutingRule>,
}

/// Centralized LLM scheduler with purpose-based routing.
pub struct LlmScheduler {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    routing: HashMap<LlmPurpose, String>,  // purpose -> provider_name
    default_provider: String,
}

impl LlmScheduler {
    /// Create a scheduler directly from pre-built providers and routing rules.
    ///
    /// Useful for testing with mock providers.
    pub fn from_providers(
        providers: HashMap<String, Arc<dyn LlmProvider>>,
        routing: HashMap<LlmPurpose, String>,
    ) -> Self {
        let default_provider = providers.keys().next().cloned().unwrap_or_default();
        Self {
            providers,
            routing,
            default_provider,
        }
    }

    /// Create a new scheduler from config.
    pub fn new(config: &SchedulerConfig) -> Result<Self> {
        let mut providers = HashMap::new();
        for pc in &config.providers {
            let provider_config = ProviderConfig {
                name: pc.name.clone(),
                base_url: pc.base_url.clone(),
                api_key: resolve_api_key(&pc.api_key, &pc.name),
                transport: match pc.kind.as_str() {
                    "anthropic" => Transport::Anthropic,
                    "ollama" => Transport::Openai,
                    _ => Transport::Openai,
                },
                models: vec![pc.model.clone()],
            };
            let provider = create_provider_by_kind(&pc.kind, &provider_config, &pc.model)?;
            providers.insert(pc.name.clone(), provider);
        }

        let mut routing = HashMap::new();
        for rule in &config.routing {
            routing.insert(rule.purpose.clone(), rule.provider_name.clone());
        }

        let default_provider = config.providers.first()
            .map(|p| p.name.clone())
            .unwrap_or_default();

        Ok(Self {
            providers,
            routing,
            default_provider,
        })
    }

    /// Route a purpose to a provider name.
    fn resolve_provider(&self, purpose: &LlmPurpose) -> &str {
        self.routing
            .get(purpose)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_provider)
    }

    /// Get a provider by name.
    pub fn provider(&self, name: &str) -> Option<&Arc<dyn LlmProvider>> {
        self.providers.get(name)
    }

    /// Execute a completion request routed by purpose.
    pub async fn complete(
        &self,
        purpose: &LlmPurpose,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let provider_name = self.resolve_provider(purpose);
        let provider = self.providers.get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;
        provider.complete(messages, tools).await
    }

    /// Get the provider for task execution (Engine use).
    pub fn executor_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Execute);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers.values().next().expect("No LLM providers configured")
        })
    }

    /// Get the provider for reflection (BrainCore use).
    pub fn reflector_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Reflect);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers.values().next().expect("No LLM providers configured")
        })
    }

    /// Check health of the default provider.
    ///
    /// Phase 1: returns basic status (always available).
    /// Phase 2: actually ping providers and measure latency.
    pub async fn health_check(&self) -> ProviderHealth {
        ProviderHealth {
            name: self.default_provider.clone(),
            available: true,
            latency_ms: 0,
            tokens_remaining: None,
        }
    }
}

/// Resolve API key: config value first, then env var `<NAME>_API_KEY`.
fn resolve_api_key(api_key: &str, provider_name: &str) -> String {
    if !api_key.is_empty() {
        return api_key.to_string();
    }
    let env_name = format!(
        "{}_API_KEY",
        provider_name.to_uppercase().replace('-', "_")
    );
    std::env::var(&env_name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_api_key_from_config() {
        assert_eq!(resolve_api_key("sk-secret", "test"), "sk-secret");
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        // When api_key is empty, falls back to env var
        let result = resolve_api_key("", "nonexistent_provider_xyz");
        assert_eq!(result, ""); // env var not set
    }

    #[test]
    fn test_scheduler_config_construction() {
        let config = SchedulerConfig {
            providers: vec![
                SchedulerProviderConfig {
                    name: "executor".to_string(),
                    base_url: "https://api.openai.com".to_string(),
                    api_key: "sk-test".to_string(),
                    kind: "openai".to_string(),
                    model: "gpt-4o".to_string(),
                },
            ],
            routing: vec![
                RoutingRule {
                    purpose: LlmPurpose::Execute,
                    provider_name: "executor".to_string(),
                },
            ],
        };
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.routing.len(), 1);
    }
}
