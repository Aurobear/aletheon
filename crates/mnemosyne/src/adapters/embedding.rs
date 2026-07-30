//! Endpoint-pinned remote embedding adapters.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context};
use async_trait::async_trait;
use fabric::memory::ProviderBackpressurePort;
use fabric::{Clock, EmbeddingProvider};
use serde::Deserialize;

use crate::credential::EmbeddingCredentialGrant;

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
        let provider_key = format!("embedding:{}:{}", grant.provider_id, model);
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
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                EmbeddingAdapterError::Timeout
            } else {
                EmbeddingAdapterError::ProviderUnavailable
            }
        })?;
        if !response.status().is_success() {
            let retry_after_ms = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1_000));
            self.backpressure
                .observe_retry_after(&self.provider_key, retry_after_ms);
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
