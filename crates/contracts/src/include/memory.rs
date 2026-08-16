//! Dependency-neutral provider admission and embedding contracts.

use anyhow::Result;
use async_trait::async_trait;

/// Trait for text embedding models.
///
/// Implementations convert a text string into a dense float vector
/// suitable for cosine similarity search.  The ABI crate defines only
/// the contract; concrete providers (OpenAI, local models, hash-based
/// fallbacks) live in higher-level crates.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a text string into a vector of floats.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a bounded batch. Providers may override this with one transport
    /// request; the default preserves compatibility for local/test adapters.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }
        Ok(embeddings)
    }

    /// Stable provenance identity used to detect stale vector indexes.
    fn model_id(&self) -> &str {
        "unknown"
    }

    /// Return the dimension of the embedding vectors.
    fn dimension(&self) -> usize;
}

/// Opaque admission lease retained for one provider request.
pub trait ProviderRequestPermit: Send + Sync {}
impl<T: Send + Sync> ProviderRequestPermit for T {}

/// Stable machine-scoped identity shared by LLM and embedding callers that
/// target the same provider endpoint and model.
pub fn provider_backpressure_key(endpoint: &str, model: &str) -> String {
    format!(
        "{}::{}",
        endpoint.trim().trim_end_matches('/'),
        model.trim()
    )
}

/// Conservative shared cooldown when a transient provider failure omits
/// `Retry-After`. The machine authority still caps this against the effective
/// provider's `max_cooldown_ms`.
pub const DEFAULT_TRANSIENT_PROVIDER_COOLDOWN_MS: u64 = 30_000;

/// Wire-safe observability snapshot for machine-scoped provider admission.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderBackpressureSnapshot {
    pub admitted: u64,
    pub queued: u64,
    #[serde(default)]
    pub paced: u64,
    pub rejected: u64,
    pub cooldown_updates: u64,
    pub active: usize,
    pub available_permits: usize,
}

/// Dependency-neutral machine/provider admission boundary used by remote
/// embedding adapters as well as LLM transports.
#[async_trait]
pub trait ProviderBackpressurePort: Send + Sync {
    async fn acquire(&self, provider_key: &str) -> Result<Box<dyn ProviderRequestPermit>>;
    async fn observe_retry_after(&self, provider_key: &str, retry_after_ms: Option<u64>);
}

#[cfg(test)]
mod provider_backpressure_key_tests {
    use super::provider_backpressure_key;

    #[test]
    fn key_normalizes_outer_space_and_trailing_endpoint_slashes() {
        assert_eq!(
            provider_backpressure_key(" https://provider.example/v1/// ", " model-a "),
            "https://provider.example/v1::model-a"
        );
    }
}
