//! Canonical provider definition resolution and construction.

use std::sync::Arc;

use anyhow::Result;

use super::anthropic::AnthropicProvider;
use super::ollama::OllamaProvider;
use super::openai_provider::OpenAiProvider;
use super::provider::LlmProvider;
use crate::config::{ProviderConfig, ProviderPricing, ProviderTimeoutConfig, Transport};

/// Concrete protocol selected after resolving the compatibility-only `Auto` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Ollama,
}

/// Runtime parameters applied by the single provider construction path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderBuildOptions {
    pub max_tokens: u32,
    pub timeouts: ProviderTimeoutConfig,
}

impl Default for ProviderBuildOptions {
    fn default() -> Self {
        Self {
            max_tokens: 100_000,
            timeouts: ProviderTimeoutConfig::default(),
        }
    }
}

/// Non-secret interpretation of the canonical provider definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProviderDefinition {
    pub kind: ProviderKind,
    pub credential_env_name: String,
    pub max_context_length: Option<usize>,
    pub pricing: Option<ProviderPricing>,
}

/// Resolve the transport and deployment metadata without constructing a client.
///
/// Explicit transport is authoritative. `Auto` is retained only for backwards
/// compatibility with existing endpoint-based configuration.
pub fn resolve_provider_definition(config: &ProviderConfig) -> ResolvedProviderDefinition {
    let kind = match config.transport {
        Transport::Openai => ProviderKind::OpenAi,
        Transport::Anthropic => ProviderKind::Anthropic,
        Transport::Ollama => ProviderKind::Ollama,
        Transport::Auto => detect_compatibility_kind(&config.base_url),
    };
    ResolvedProviderDefinition {
        kind,
        credential_env_name: credential_env_name(&config.name),
        max_context_length: config.max_context_length,
        pricing: config.pricing.clone(),
    }
}

/// The only production implementation that constructs an LLM provider.
pub fn create_provider(
    config: &ProviderConfig,
    model: &str,
    options: ProviderBuildOptions,
) -> Result<Arc<dyn LlmProvider>> {
    let resolved = resolve_provider_definition(config);
    let api_key = resolve_api_key(config, &resolved.credential_env_name);

    match resolved.kind {
        ProviderKind::Anthropic => {
            let mut provider = AnthropicProvider::new(&api_key, model)
                .with_base_url(&config.base_url)
                .with_timeouts(options.timeouts)
                .with_max_tokens(options.max_tokens);
            if let Some(context) = resolved.max_context_length {
                provider = provider.with_max_context(context);
            }
            Ok(Arc::new(provider))
        }
        ProviderKind::OpenAi => {
            let mut provider = OpenAiProvider::new(&api_key, model, &config.base_url)
                .with_timeouts(options.timeouts)
                .with_max_tokens(options.max_tokens);
            if let Some(context) = resolved.max_context_length {
                provider = provider.with_max_context(context);
            }
            Ok(Arc::new(provider))
        }
        ProviderKind::Ollama => {
            let mut provider = OllamaProvider::new(model)
                .with_base_url(&config.base_url)
                .with_timeouts(options.timeouts)?
                .with_max_tokens(options.max_tokens);
            if let Some(context) = resolved.max_context_length {
                provider = provider.with_max_context(context);
            }
            Ok(Arc::new(provider))
        }
    }
}

fn detect_compatibility_kind(base_url: &str) -> ProviderKind {
    let normalized = base_url.trim().to_ascii_lowercase();
    if normalized.ends_with("/anthropic") {
        ProviderKind::Anthropic
    } else if normalized.contains("localhost:11434") || normalized.contains("127.0.0.1:11434") {
        ProviderKind::Ollama
    } else {
        ProviderKind::OpenAi
    }
}

fn credential_env_name(provider_name: &str) -> String {
    format!(
        "{}_API_KEY",
        provider_name.to_ascii_uppercase().replace('-', "_")
    )
}

fn resolve_api_key(config: &ProviderConfig, env_name: &str) -> String {
    if !config.api_key.is_empty() {
        return config.api_key.clone();
    }
    std::env::var(env_name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(transport: Transport, base_url: &str) -> ProviderConfig {
        ProviderConfig {
            name: "local-provider".into(),
            base_url: base_url.into(),
            api_key: "test-key".into(),
            transport,
            models: vec!["model".into()],
            max_context_length: Some(32_768),
            pricing: Some(ProviderPricing {
                input_per_1k: 0.1,
                output_per_1k: 0.2,
            }),
        }
    }

    #[test]
    fn explicit_transport_is_authoritative() {
        assert_eq!(
            resolve_provider_definition(&definition(
                Transport::Openai,
                "http://localhost:11434/anthropic"
            ))
            .kind,
            ProviderKind::OpenAi
        );
        assert_eq!(
            resolve_provider_definition(&definition(
                Transport::Ollama,
                "https://api.example.com/anthropic"
            ))
            .kind,
            ProviderKind::Ollama
        );
    }

    #[test]
    fn auto_endpoint_heuristic_is_compatibility_only_and_deterministic() {
        for (url, expected) in [
            ("https://api.example.com/anthropic", ProviderKind::Anthropic),
            ("http://localhost:11434", ProviderKind::Ollama),
            ("http://127.0.0.1:11434", ProviderKind::Ollama),
            ("https://api.openai.com", ProviderKind::OpenAi),
        ] {
            assert_eq!(
                resolve_provider_definition(&definition(Transport::Auto, url)).kind,
                expected
            );
        }
    }

    #[test]
    fn canonical_resolution_carries_runtime_metadata_and_credential_identity() {
        let resolved = resolve_provider_definition(&definition(
            Transport::Anthropic,
            "https://api.anthropic.com",
        ));
        assert_eq!(resolved.credential_env_name, "LOCAL_PROVIDER_API_KEY");
        assert_eq!(resolved.max_context_length, Some(32_768));
        assert_eq!(resolved.pricing.unwrap().output_per_1k, 0.2);
    }

    #[test]
    fn canonical_factory_builds_each_explicit_protocol() {
        for transport in [Transport::Openai, Transport::Anthropic, Transport::Ollama] {
            let provider = create_provider(
                &definition(transport, "http://127.0.0.1:11434"),
                "model",
                ProviderBuildOptions::default(),
            )
            .unwrap();
            assert_eq!(provider.name(), "model");
        }
    }
}
