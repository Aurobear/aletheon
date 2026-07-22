//! Typed construction unit for the daemon inference boundary.

use std::sync::Arc;

use fabric::LlmProvider;

use crate::service::inference_port::{InferencePort, PortLlmProvider};

pub(super) struct InferenceCompositionInput {
    pub(super) port: Arc<dyn InferencePort>,
    pub(super) model_spec: String,
}

pub(super) struct InferenceComposition {
    pub(super) provider: Arc<dyn LlmProvider>,
}

pub(super) fn compose(input: InferenceCompositionInput) -> InferenceComposition {
    InferenceComposition {
        provider: Arc::new(PortLlmProvider::new(input.port, input.model_spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::inference_port::{CoreInferenceRequest, InferenceError};
    use fabric::{ContentBlock, LlmResponse, LlmStream, Message, StopReason, Usage};

    struct RecordingPort;

    #[async_trait::async_trait]
    impl InferencePort for RecordingPort {
        async fn complete(
            &self,
            request: CoreInferenceRequest,
        ) -> Result<LlmResponse, InferenceError> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: request.model_spec,
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
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
        });

        assert_eq!(composition.provider.name(), "reviewed/model");
        let response = composition
            .provider
            .complete(&[Message::user("hello")], &[])
            .await
            .unwrap();
        assert!(matches!(
            response.content.as_slice(),
            [ContentBlock::Text { text }] if text == "reviewed/model"
        ));
    }
}
