//! Inference boundary between user-owned execution and model providers.

use std::sync::Arc;

use fabric::{LlmProvider, LlmResponse, LlmStream, Message, ModelRuntimeFacts, ToolDefinition};
use serde::{Deserialize, Serialize};

/// Wire-safe model input. Filesystem and operating-system authority are
/// intentionally absent from this frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreInferenceRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub model_spec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub model_spec: String,
    pub display_name: String,
    pub max_context_tokens: usize,
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
    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        Err(anyhow::anyhow!("model capabilities are unavailable for '{model_spec}'").into())
    }

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
    pub fn new(
        inference: Arc<dyn InferencePort>,
        capabilities: ModelCapabilities,
    ) -> anyhow::Result<Self> {
        if capabilities.max_context_tokens == 0 {
            anyhow::bail!(
                "model '{}' reported a zero-token context window",
                capabilities.model_spec
            );
        }
        Ok(Self {
            inference,
            model_spec: capabilities.model_spec,
            display_name: capabilities.display_name,
            max_context: capabilities.max_context_tokens,
        })
    }

    pub async fn resolve(
        inference: Arc<dyn InferencePort>,
        model_spec: impl AsRef<str>,
    ) -> anyhow::Result<Self> {
        let capabilities = inference.capabilities(model_spec.as_ref()).await?;
        Self::new(inference, capabilities)
    }

    pub(crate) fn model_spec(&self) -> &str {
        &self.model_spec
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

    fn runtime_facts(&self) -> ModelRuntimeFacts {
        ModelRuntimeFacts {
            effective_model_id: self.model_spec.clone(),
            display_name: self.display_name.clone(),
            max_context_tokens: self.max_context,
        }
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
    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        let display_name = self.provider.name().to_string();
        Ok(ModelCapabilities {
            model_spec: if model_spec.trim().is_empty() {
                display_name.clone()
            } else {
                model_spec.trim().to_string()
            },
            display_name,
            max_context_tokens: self.provider.max_context_length(),
        })
    }

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
