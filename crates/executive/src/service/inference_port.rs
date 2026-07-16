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
