//! Canonical provider definition resolution and construction.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use fabric::{
    memory::DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS, InferenceCapabilities, LlmResponse, LlmStream,
    Message, ModelRuntimeFacts, ToolDefinition,
};
use futures::StreamExt;

use crate::adapters::inference::anthropic::AnthropicProvider;
use crate::adapters::inference::ollama::OllamaProvider;
use crate::adapters::inference::openai_provider::OpenAiProvider;
use crate::adapters::inference::provider::LlmProvider;
use crate::adapters::inference::{
    backpressure,
    provider::{InferenceFailure, InferenceFailureKind},
};
use crate::config::{ProviderConfig, ProviderPricing, ProviderTimeoutConfig, Transport};

use super::model_catalog;

/// Concrete protocol selected after resolving the compatibility-only `Auto` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderKind {
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
pub(crate) struct ResolvedProviderDefinition {
    pub kind: ProviderKind,
    pub credential_env_name: String,
    pub max_context_length: Option<usize>,
    pub pricing: Option<ProviderPricing>,
}

/// Resolve the transport and deployment metadata without constructing a client.
///
/// Explicit transport is authoritative. `Auto` fails closed; endpoint values
/// are never inspected to infer an adapter.
pub(crate) fn resolve_provider_definition(
    config: &ProviderConfig,
) -> Result<ResolvedProviderDefinition> {
    let kind = match config.transport {
        Transport::Openai => ProviderKind::OpenAi,
        Transport::Anthropic => ProviderKind::Anthropic,
        Transport::Ollama => ProviderKind::Ollama,
        Transport::Auto => anyhow::bail!("provider transport must be explicit"),
    };
    Ok(ResolvedProviderDefinition {
        kind,
        credential_env_name: credential_env_name(&config.name),
        max_context_length: config.max_context_length,
        pricing: config.pricing.clone(),
    })
}

/// The only production implementation that constructs an LLM provider.
pub fn create_provider(
    config: &ProviderConfig,
    model: &str,
    options: ProviderBuildOptions,
) -> Result<Arc<dyn LlmProvider>> {
    let resolved = resolve_provider_definition(config)?;
    let api_key = resolve_api_key(config, &resolved.credential_env_name);
    let model_spec = model_catalog::resolve_spec(model, resolved.max_context_length)?;
    let max_tokens = model_spec
        .max_output_tokens
        .and_then(|limit| u32::try_from(limit).ok())
        .map_or(options.max_tokens, |limit| options.max_tokens.min(limit));

    let provider: Arc<dyn LlmProvider> = match resolved.kind {
        ProviderKind::Anthropic => {
            let provider = AnthropicProvider::new(&api_key, &model_spec.wire_id)
                .with_base_url(&config.base_url)
                .with_timeouts(options.timeouts)
                .with_max_tokens(max_tokens)
                .with_max_context(model_spec.context_window_tokens);
            Arc::new(provider)
        }
        ProviderKind::OpenAi => {
            let provider = OpenAiProvider::new(&api_key, &model_spec.wire_id, &config.base_url)
                .with_timeouts(options.timeouts)
                .with_max_tokens(max_tokens)
                .with_max_context(model_spec.context_window_tokens);
            Arc::new(provider)
        }
        ProviderKind::Ollama => {
            let provider = OllamaProvider::new(&model_spec.wire_id)
                .with_base_url(&config.base_url)
                .with_timeouts(options.timeouts)?
                .with_max_tokens(max_tokens)
                .with_max_context(model_spec.context_window_tokens);
            Arc::new(provider)
        }
    };
    Ok(Arc::new(BackpressuredProvider {
        inner: provider,
        state: backpressure::state_for(
            &fabric::memory::provider_backpressure_key(&config.base_url, &model_spec.wire_id),
            config.backpressure,
        ),
    }))
}

struct BackpressuredProvider {
    inner: Arc<dyn LlmProvider>,
    state: Arc<backpressure::ProviderState>,
}

fn observe_failure(state: &backpressure::ProviderState, error: &anyhow::Error) {
    let retry_after = error
        .chain()
        .find_map(|source| source.downcast_ref::<InferenceFailure>())
        .and_then(|failure| {
            failure.retry_after_ms.or_else(|| {
                (failure.kind == InferenceFailureKind::Transient
                    && failure.code == "provider_unavailable")
                    .then_some(DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS)
            })
        });
    backpressure::observe_retry_after(state, retry_after);
}

#[async_trait]
impl LlmProvider for BackpressuredProvider {
    fn capabilities(&self) -> InferenceCapabilities {
        self.inner.capabilities()
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let _permit = backpressure::acquire(&self.state).await?;
        let result = self.inner.complete(messages, tools).await;
        if let Err(error) = &result {
            observe_failure(&self.state, error);
        }
        result
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmStream> {
        let permit = backpressure::acquire(&self.state).await?;
        match self.inner.complete_stream(messages, tools).await {
            Ok(stream) => {
                let state = self.state.clone();
                Ok(Box::pin(stream.map(move |item| {
                    let _keep_permit_alive = &permit;
                    if let Err(error) = &item {
                        observe_failure(&state, error);
                    }
                    item
                })))
            }
            Err(error) => {
                observe_failure(&self.state, &error);
                Err(error)
            }
        }
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
    fn runtime_facts(&self) -> ModelRuntimeFacts {
        self.inner.runtime_facts()
    }
    fn max_context_length(&self) -> usize {
        self.inner.max_context_length()
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
            backpressure: Default::default(),
        }
    }

    #[test]
    fn explicit_transport_is_authoritative() {
        assert_eq!(
            resolve_provider_definition(&definition(
                Transport::Openai,
                "http://localhost:11434/anthropic"
            ))
            .unwrap()
            .kind,
            ProviderKind::OpenAi
        );
        assert_eq!(
            resolve_provider_definition(&definition(
                Transport::Ollama,
                "https://api.example.com/anthropic"
            ))
            .unwrap()
            .kind,
            ProviderKind::Ollama
        );
    }

    #[test]
    fn auto_transport_fails_closed_without_url_inference() {
        assert!(resolve_provider_definition(&definition(
            Transport::Auto,
            "http://localhost:11434/anthropic",
        ))
        .is_err());
    }

    #[test]
    fn canonical_resolution_carries_runtime_metadata_and_credential_identity() {
        let resolved = resolve_provider_definition(&definition(
            Transport::Anthropic,
            "https://api.anthropic.com",
        ))
        .unwrap();
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

    #[test]
    fn catalog_derives_context_without_rewriting_provider_model_id() {
        let mut config = definition(Transport::Openai, "https://aiapi.lejurobot.com");
        config.max_context_length = None;
        let provider = create_provider(
            &config,
            "deepseek/deepseek-v4-flash[1m]",
            ProviderBuildOptions::default(),
        )
        .unwrap();
        assert_eq!(provider.name(), "deepseek/deepseek-v4-flash");
        assert_eq!(provider.max_context_length(), 1_000_000);
    }

    #[test]
    fn known_model_rejects_conflicting_manual_context_override() {
        let config = definition(Transport::Openai, "https://aiapi.lejurobot.com");
        let error = create_provider(
            &config,
            "deepseek/deepseek-v4-flash[512k]",
            ProviderBuildOptions::default(),
        )
        .err()
        .unwrap();
        assert!(error
            .to_string()
            .contains("conflicts with the model catalog"));
    }

    #[test]
    fn transient_provider_failure_without_retry_after_still_sets_shared_cooldown() {
        let key = format!("fallback-cooldown-{}", uuid::Uuid::new_v4());
        let state = backpressure::state_for(&key, Default::default());

        observe_failure(&state, &InferenceFailure::transient("provider_unavailable"));

        assert_eq!(
            backpressure::provider_backpressure_snapshot(&key)
                .unwrap()
                .cooldown_updates,
            1
        );
    }
}
