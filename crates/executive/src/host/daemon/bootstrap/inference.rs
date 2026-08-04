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
