use std::collections::HashMap;
use std::sync::Arc;

use crate::composition::inference_factory::{create_provider, ProviderBuildOptions};
use super::llm::LlmProvider;
use crate::config::{CognitConfig, ProviderConfig, ProviderTimeoutConfig};

/// Registry of configured providers.
#[derive(Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, ProviderConfig>,
    aliases: HashMap<String, (String, String)>, // alias → (provider_name, model)
    default_provider: String,
    default_model: String,
    max_tokens: u32,
    provider_timeouts: ProviderTimeoutConfig,
}

impl ProviderRegistry {
    /// Build the registry from Executive's validated Cognit domain view.
    pub fn from_config(config: &CognitConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let mut providers = HashMap::new();
        for p in &config.providers {
            providers.insert(p.name.clone(), p.clone());
        }

        // Resolve aliases: "sonnet" → ("anthropic", "claude-sonnet-4-20250514")
        let mut aliases = HashMap::new();
        for (alias, spec) in &config.model_aliases {
            if let Some((provider, model)) = spec.split_once('/') {
                aliases.insert(alias.clone(), (provider.to_string(), model.to_string()));
            }
        }

        let default_provider = config.agent.default_provider.clone().unwrap_or_else(|| {
            config
                .providers
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_default()
        });

        let default_model = config.agent.default_model.clone().unwrap_or_else(|| {
            config
                .providers
                .first()
                .and_then(|p| p.models.first().cloned())
                .unwrap_or_default()
        });

        Ok(Self {
            providers,
            aliases,
            default_provider,
            default_model,
            max_tokens: config.agent.max_tokens as u32,
            provider_timeouts: config.agent.provider_timeouts,
        })
    }

    /// Resolve a model spec to (provider_config, model_name).
    ///
    /// Formats supported:
    /// - "provider/model" → explicit provider and model
    /// - "alias" → resolved via model_aliases
    /// - "model" → uses default_provider
    /// - "" → uses default_provider and default_model
    pub fn resolve(&self, spec: &str) -> anyhow::Result<(ProviderConfig, String)> {
        let spec = spec.trim();

        if spec.is_empty() {
            // Use defaults
            let provider = self
                .providers
                .get(&self.default_provider)
                .ok_or_else(|| {
                    anyhow::anyhow!("Default provider '{}' not found", self.default_provider)
                })?
                .clone();
            return Ok((provider, self.default_model.clone()));
        }

        // Try "provider/model" format
        if let Some((provider_name, model)) = spec.split_once('/') {
            let provider = self
                .providers
                .get(provider_name)
                .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?
                .clone();
            return Ok((provider, model.to_string()));
        }

        // Try alias
        if let Some((provider_name, model)) = self.aliases.get(spec) {
            let provider = self
                .providers
                .get(provider_name)
                .ok_or_else(|| {
                    anyhow::anyhow!("Provider '{}' not found (alias '{}')", provider_name, spec)
                })?
                .clone();
            return Ok((provider, model.clone()));
        }

        // Try as model name with default provider
        let provider = self
            .providers
            .get(&self.default_provider)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", self.default_provider))?
            .clone();
        Ok((provider, spec.to_string()))
    }

    /// Resolve a configured role route without silently treating an unknown
    /// alias as a model on the default provider.
    pub fn resolve_role_alias(&self, spec: &str) -> anyhow::Result<(ProviderConfig, String)> {
        let spec = spec.trim();
        if spec.is_empty() {
            anyhow::bail!("role runtime model alias must not be empty");
        }
        if spec.contains('/') || self.aliases.contains_key(spec) {
            return self.resolve(spec);
        }
        anyhow::bail!("model alias '{}' not found", spec)
    }

    /// Create an LLM provider through the canonical factory.
    pub fn create_provider(
        &self,
        config: &ProviderConfig,
        model: &str,
    ) -> anyhow::Result<Arc<dyn LlmProvider>> {
        create_provider(
            config,
            model,
            ProviderBuildOptions {
                max_tokens: self.max_tokens,
                timeouts: self.provider_timeouts,
            },
        )
    }

    /// Resolve and create a provider in one step.
    pub fn resolve_and_create(&self, spec: &str) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let (config, model) = self.resolve(spec)?;
        self.create_provider(&config, &model)
    }

    /// Get the default model spec (provider/model format).
    pub fn default_spec(&self) -> String {
        format!("{}/{}", self.default_provider, self.default_model)
    }

    /// List all configured provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> CognitConfig {
        let toml = r#"
[agent]
default_provider = "mimo"
default_model = "mimo-v2.5-pro"

[[providers]]
name = "mimo"
base_url = "https://token-plan-sgp.xiaomimimo.com"
api_key = "tp-test"
transport = "openai"
models = ["mimo-v2.5-pro", "mimo-v2.5-flash"]

[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "sk-ant-test"
transport = "anthropic"
models = ["claude-sonnet-4-20250514"]

[[providers]]
name = "ollama"
base_url = "http://localhost:11434"
api_key = ""
transport = "openai"
models = ["qwen3:8b"]

[model_aliases]
sonnet = "anthropic/claude-sonnet-4-20250514"
local = "ollama/qwen3:8b"
"#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn test_resolve_explicit() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        let (provider, model) = registry
            .resolve("anthropic/claude-sonnet-4-20250514")
            .unwrap();
        assert_eq!(provider.name, "anthropic");
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_resolve_alias() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        let (provider, model) = registry.resolve("sonnet").unwrap();
        assert_eq!(provider.name, "anthropic");
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_resolve_default() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        let (provider, model) = registry.resolve("").unwrap();
        assert_eq!(provider.name, "mimo");
        assert_eq!(model, "mimo-v2.5-pro");
    }

    #[test]
    fn test_resolve_model_only() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        let (provider, model) = registry.resolve("mimo-v2.5-flash").unwrap();
        assert_eq!(provider.name, "mimo");
        assert_eq!(model, "mimo-v2.5-flash");
    }

    #[test]
    fn test_resolve_unknown_provider() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        assert!(registry.resolve("unknown/model").is_err());
    }

    #[test]
    fn test_default_spec() {
        let config = make_config();
        let registry = ProviderRegistry::from_config(&config).unwrap();

        assert_eq!(registry.default_spec(), "mimo/mimo-v2.5-pro");
    }
}
