//! D2 Cognit `CognitiveRun` + `InferencePort` owner seam (Agent Kernel V2).
//!
//! Splits the provider adapter from Cognit core's default dependency surface.
//! `InferencePort` is the provider-neutral inference boundary; `CognitiveRun`
//! is the narrow run port Robot/Executor consumes.  Runtime takes over loop
//! driving and settlement at the cutover.  Cognit core must not depend on
//! Robot concrete types here (Robot files migrate at E5).  No writer cutover.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Provider-neutral inference request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub prompt: String,
    pub model: Option<String>,
}

/// Provider-neutral inference response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceResponse {
    pub text: String,
}

/// Provider-neutral inference boundary.  Implemented by a provider adapter;
/// Cognit core depends on this port, not on a concrete provider.
#[async_trait]
pub trait InferencePort: Send + Sync {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError>;
}

/// Typed inference errors — provider rejection/timeout fail closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InferenceError {
    #[error("provider unavailable")]
    ProviderUnavailable,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("inference timed out")]
    Timeout,
}

/// Narrow run port Robot/Executor consumes (D2: "只切出 Robot 所需的
/// `CognitiveRun` port").  The Runtime drives the loop and settlement.
#[async_trait]
pub trait CognitiveRun: Send + Sync {
    async fn run(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError>;
}

/// Adapter from `InferencePort` to `CognitiveRun` for owner-local use.
pub struct InferenceToCognitiveRun {
    inference: std::sync::Arc<dyn InferencePort>,
}

impl InferenceToCognitiveRun {
    pub fn new(inference: std::sync::Arc<dyn InferencePort>) -> Self {
        Self { inference }
    }
}

#[async_trait]
impl CognitiveRun for InferenceToCognitiveRun {
    async fn run(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        self.inference.complete(request).await
    }
}

/// Test provider.
#[derive(Default)]
pub struct EchoProvider;

#[async_trait]
impl InferencePort for EchoProvider {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        Ok(InferenceResponse {
            text: format!("echo:{}", request.prompt),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cognitive_run_delegates_to_inference_port() {
        let run = InferenceToCognitiveRun::new(std::sync::Arc::new(EchoProvider));
        let resp = run
            .run(InferenceRequest {
                prompt: "hello".into(),
                model: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.text, "echo:hello");
    }
}
