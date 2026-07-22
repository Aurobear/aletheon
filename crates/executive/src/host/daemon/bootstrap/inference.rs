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

pub(super) fn compose(input: InferenceCompositionInput) -> InferenceComposition {
    InferenceComposition {
        provider: Arc::new(PortLlmProvider::new(input.port, input.model_spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::inference_port::{CoreInferenceRequest, InferenceError};
    use fabric::{LlmResponse, LlmStream};

    struct RecordingPort;

    #[async_trait::async_trait]
    impl InferencePort for RecordingPort {
        async fn complete(&self, _: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
            Err(anyhow::anyhow!("completion disabled in construction fixture").into())
        }

        async fn stream(&self, _: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
            Err(anyhow::anyhow!("stream disabled in fixture").into())
        }
    }

    #[test]
    fn binds_the_injected_port_and_model_without_environment_lookup() {
        let composition = compose(InferenceCompositionInput {
            port: Arc::new(RecordingPort),
            model_spec: "reviewed/model".into(),
        });

        assert_eq!(composition.provider.name(), "reviewed/model");
        assert_eq!(composition.provider.max_context_length(), 128_000);
    }
}
