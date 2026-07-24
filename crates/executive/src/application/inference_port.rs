//! Inference boundary between user-owned execution and model providers.

use std::sync::Arc;

use fabric::{LlmProvider, LlmResponse, LlmStream, Message, ToolDefinition};
use serde::{Deserialize, Serialize};

/// Wire-safe model input. Filesystem and operating-system authority are
/// intentionally absent from this frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreInferenceRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub model_spec: String,
}

#[derive(Debug, thiserror::Error)]
#[error("inference provider failed: {0}")]
pub struct InferenceError(#[source] anyhow::Error);

impl From<anyhow::Error> for InferenceError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

/// Object-safe inference operations used by the user runtime.
#[async_trait::async_trait]
pub trait InferencePort: Send + Sync {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError>;

    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError>;
}

/// Compatibility adapter that delegates to an in-process provider.
pub struct LocalInferencePort {
    provider: Arc<dyn LlmProvider>,
}

/// Presents one model selection on an `InferencePort` as the legacy provider
/// interface consumed by Cognit sessions. Provider credentials remain behind
/// the port; only the model specification crosses the boundary.
#[derive(Clone)]
pub struct PortLlmProvider {
    inference: Arc<dyn InferencePort>,
    model_spec: String,
    display_name: String,
    max_context: usize,
}

impl PortLlmProvider {
    pub fn new(inference: Arc<dyn InferencePort>, model_spec: impl Into<String>) -> Self {
        let model_spec = model_spec.into();
        let display_name = if model_spec.is_empty() {
            "core-default".to_string()
        } else {
            model_spec.clone()
        };
        Self {
            inference,
            model_spec,
            display_name,
            max_context: 128_000,
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for PortLlmProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.inference
            .complete(CoreInferenceRequest {
                messages: messages.to_vec(),
                tools: tools.to_vec(),
                model_spec: self.model_spec.clone(),
            })
            .await
            .map_err(anyhow::Error::from)
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        self.inference
            .stream(CoreInferenceRequest {
                messages: messages.to_vec(),
                tools: tools.to_vec(),
                model_spec: self.model_spec.clone(),
            })
            .await
            .map_err(anyhow::Error::from)
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    fn max_context_length(&self) -> usize {
        self.max_context
    }
}

impl LocalInferencePort {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl InferencePort for LocalInferencePort {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        self.provider
            .complete(&request.messages, &request.tools)
            .await
            .map_err(Into::into)
    }

    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        self.provider
            .complete_stream(&request.messages, &request.tools)
            .await
            .map_err(Into::into)
    }
}
