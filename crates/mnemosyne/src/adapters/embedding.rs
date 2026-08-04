//! Endpoint-pinned remote embedding adapters.
#![allow(clippy::items_after_test_module)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context};
use async_trait::async_trait;
use fabric::memory::{ProviderBackpressurePort, DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS};
use fabric::{Clock, EmbeddingProvider};
use serde::Deserialize;

use crate::credential::EmbeddingCredentialGrant;

fn retry_after_ms(status: reqwest::StatusCode, value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1_000))
        .or_else(|| {
            (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                .then_some(DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS)
        })
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingAdapterError {
    #[error("embedding_endpoint_untrusted")]
    EndpointUntrusted,
    #[error("embedding_timeout")]
    Timeout,
    #[error("embedding_provider_unavailable")]
    ProviderUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingTransport {
    OpenAi,
    Ollama,
}

pub struct RemoteEmbeddingProvider {
    transport: EmbeddingTransport,
    base_url: String,
    model: String,
    dimension: usize,
    grant: EmbeddingCredentialGrant,
    clock: Arc<dyn Clock>,
    backpressure: Arc<dyn ProviderBackpressurePort>,
    client: reqwest::Client,
    provider_key: String,
}

impl RemoteEmbeddingProvider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: EmbeddingTransport,
        base_url: impl Into<String>,
        model: impl Into<String>,
        dimension: usize,
        grant: EmbeddingCredentialGrant,
        clock: Arc<dyn Clock>,
        backpressure: Arc<dyn ProviderBackpressurePort>,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        ensure!(dimension > 0, "embedding dimension must be positive");
        let base_url = base_url.into();
        let model = model.into();
        let provider_key = fabric::memory::provider_backpressure_key(&base_url, &model);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .build()?;
        Ok(Self {
            transport,
            base_url,
            model,
            dimension,
            grant,
            clock,
            backpressure,
            client,
            provider_key,
        })
    }

    async fn request(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let now = self.clock.wall_now().0.max(0) as u64 / 1_000;
        let secret = self
            .grant
            .secret_if_approved(&self.base_url, now)
            .ok_or(EmbeddingAdapterError::EndpointUntrusted)?;
        let _permit = self.backpressure.acquire(&self.provider_key).await?;
        let endpoint = match self.transport {
            EmbeddingTransport::OpenAi => {
                format!("{}/embeddings", self.base_url.trim_end_matches('/'))
            }
            EmbeddingTransport::Ollama => {
                format!("{}/api/embed", self.base_url.trim_end_matches('/'))
            }
        };
        let mut request = self.client.post(endpoint).json(&serde_json::json!({
            "model": self.model,
            "input": texts,
            "dimensions": self.dimension,
        }));
        if self.transport == EmbeddingTransport::OpenAi && !secret.is_empty() {
            request = request.bearer_auth(secret);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                self.backpressure
                    .observe_retry_after(
                        &self.provider_key,
                        Some(DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS),
                    )
                    .await;
                return Err(if error.is_timeout() {
                    EmbeddingAdapterError::Timeout
                } else {
                    EmbeddingAdapterError::ProviderUnavailable
                }
                .into());
            }
        };
        if !response.status().is_success() {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let retry_after_ms = retry_after_ms(response.status(), retry_after.as_deref());
            self.backpressure
                .observe_retry_after(&self.provider_key, retry_after_ms)
                .await;
            return Err(EmbeddingAdapterError::ProviderUnavailable.into());
        }
        #[derive(Deserialize)]
        struct Datum {
            embedding: Vec<f32>,
        }
        #[derive(Deserialize)]
        struct OpenAiResponse {
            data: Vec<Datum>,
        }
        #[derive(Deserialize)]
        struct OllamaResponse {
            embeddings: Vec<Vec<f32>>,
        }
        let vectors = match self.transport {
            EmbeddingTransport::OpenAi => response
                .json::<OpenAiResponse>()
                .await?
                .data
                .into_iter()
                .map(|d| d.embedding)
                .collect(),
            EmbeddingTransport::Ollama => response.json::<OllamaResponse>().await?.embeddings,
        };
        ensure!(
            vectors.len() == texts.len(),
            "embedding response cardinality mismatch"
        );
        ensure!(
            vectors.iter().all(|v| v.len() == self.dimension),
            "embedding response dimension mismatch"
        );
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_uses_provider_advice_and_transient_fallback_only() {
        assert_eq!(
            retry_after_ms(reqwest::StatusCode::TOO_MANY_REQUESTS, Some("9")),
            Some(9_000)
        );
        assert_eq!(
            retry_after_ms(reqwest::StatusCode::TOO_MANY_REQUESTS, None),
            Some(DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS)
        );
        assert_eq!(retry_after_ms(reqwest::StatusCode::BAD_REQUEST, None), None);
    }
}

#[async_trait]
impl EmbeddingProvider for RemoteEmbeddingProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.request(&[text.to_string()])
            .await?
            .into_iter()
            .next()
            .context("empty embedding response")
    }
    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.request(texts).await
    }
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimension(&self) -> usize {
        self.dimension
    }
}
