//! Typed construction unit for the daemon inference boundary.

use std::sync::Arc;

use fabric::LlmProvider;

use crate::application::inference_port::{InferencePort, PortLlmProvider};

pub(super) struct InferenceCompositionInput {
    pub(super) port: Arc<dyn InferencePort>,
    pub(super) model_spec: String,
}

pub(super) struct InferenceComposition {
    pub(super) provider: Arc<dyn LlmProvider>,
}

pub(super) async fn compose(
    input: InferenceCompositionInput,
) -> anyhow::Result<InferenceComposition> {
    let provider = PortLlmProvider::resolve(input.port, input.model_spec).await?;
    Ok(InferenceComposition {
        provider: Arc::new(provider),
    })
}

/// Reconcile the configured model catalog with the host-observed provider
/// capabilities once at bootstrap. Downstream session construction receives a
/// verified context window instead of repeating cross-source checks.
pub(super) fn verify_runtime_context(
    provider: &dyn LlmProvider,
    configured_model_spec: &str,
) -> anyhow::Result<(fabric::ModelRuntimeFacts, usize)> {
    let runtime_facts = provider.runtime_facts();
    let context_window = runtime_facts.max_context_tokens;
    anyhow::ensure!(
        context_window > 0 && context_window == provider.max_context_length(),
        "runtime model capability conflict: runtime facts and provider context window disagree"
    );
    let resolved = cognit::composition::model_catalog::resolve_spec(
        configured_model_spec,
        Some(context_window),
    )?;
    anyhow::ensure!(
        resolved.context_window_tokens == context_window,
        "runtime model capability conflict for configured model '{}': catalog/config says {}, runtime says {}",
        configured_model_spec,
        resolved.context_window_tokens,
        context_window
    );
    Ok((runtime_facts, context_window))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::inference_port::{
        CoreInferenceRequest, InferenceError, ModelCapabilities,
    };
    use fabric::{LlmResponse, LlmStream};

    struct RecordingPort;

    #[async_trait::async_trait]
    impl InferencePort for RecordingPort {
        async fn capabilities(
            &self,
            model_spec: &str,
        ) -> Result<ModelCapabilities, InferenceError> {
            Ok(ModelCapabilities {
                provider_id: Some("fixture-provider".into()),
                transport: Some("openai".into()),
                cache_reporting: Some("deepseek_chat".into()),
                model_spec: model_spec.into(),
                display_name: model_spec.into(),
                max_context_tokens: 1_000_000,
            })
        }

        async fn complete(&self, _: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
            Err(anyhow::anyhow!("completion disabled in construction fixture").into())
        }

        async fn stream(&self, _: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
            Err(anyhow::anyhow!("stream disabled in fixture").into())
        }
    }

    #[tokio::test]
    async fn binds_the_injected_port_and_model_without_environment_lookup() {
        let composition = compose(InferenceCompositionInput {
            port: Arc::new(RecordingPort),
            model_spec: "reviewed/model".into(),
        })
        .await
        .unwrap();

        assert_eq!(composition.provider.name(), "reviewed/model");
        assert_eq!(composition.provider.max_context_length(), 1_000_000);
        let facts = composition.provider.runtime_facts();
        assert_eq!(facts.provider_id.as_deref(), Some("fixture-provider"));
        assert_eq!(facts.transport.as_deref(), Some("openai"));
        assert_eq!(facts.cache_reporting.as_deref(), Some("deepseek_chat"));
    }
}
