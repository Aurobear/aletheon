//! Vector storage abstraction for semantic memory.
//!
//! Provides a trait-based abstraction over vector databases (Qdrant, LanceDB)
//! with automatic fallback selection.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A scored result from vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredEntry {
    pub id: String,
    pub score: f32,
    pub metadata: Value,
}

/// Configuration for vector store backend selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStoreConfig {
    pub backend: VectorBackend,
    pub qdrant_url: String,
    pub lance_path: String,
    pub collection: String,
    pub dimension: usize,
    /// Optional endpoint-scoped authorization. The raw key is never serialized
    /// into configuration; callers may reveal it only after `approved_for`.
    #[serde(skip)]
    pub embedding_grant: Option<crate::credential::EmbeddingCredentialGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VectorBackend {
    Qdrant,
    Lance,
    Auto,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            backend: VectorBackend::Auto,
            qdrant_url: "http://localhost:6333".to_string(),
            lance_path: dirs::home_dir()
                .unwrap_or_else(|| "/tmp".into())
                .join(".aletheon")
                .join("vector-db")
                .to_string_lossy()
                .to_string(),
            collection: "archival_memory".to_string(),
            dimension: 384,
            embedding_grant: None,
        }
    }
}

impl VectorStoreConfig {
    /// Return a credential only for this exact configured endpoint. Redirected
    /// requests must call this again with the new origin and will fail closed.
    pub fn credential_for_endpoint(&self, request_url: &str, now_unix: u64) -> Option<&str> {
        self.embedding_grant
            .as_ref()
            .and_then(|grant| grant.secret_if_approved(request_url, now_unix))
    }
}

/// Translate the governed recall predicate into backend metadata constraints.
/// Vector implementations must attach this filter to KNN rather than filter
/// materialized results after search.
pub fn scope_predicate_filter(predicate: &crate::ScopePredicate) -> Value {
    serde_json::json!({
        "scope_key": { "$in": predicate.scope_keys },
        "sensitivity_ord": { "$lte": predicate.max_sensitivity_ord },
        "authority": { "$in": predicate.allowed_authorities },
    })
}

/// Trait for vector storage backends.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Insert or update a vector with metadata.
    async fn upsert(&self, id: &str, embedding: &[f32], metadata: Value) -> Result<()>;

    /// Search for similar vectors.
    async fn search(
        &self,
        query: &[f32],
        top_k: usize,
        filter: Option<Value>,
    ) -> Result<Vec<ScoredEntry>>;

    /// Delete a vector by ID.
    async fn delete(&self, id: &str) -> Result<()>;

    /// Count total vectors.
    async fn count(&self) -> Result<usize>;
}

// === Qdrant Implementation ===

#[cfg(feature = "vector-qdrant")]
pub struct QdrantVectorStore {
    client: reqwest::Client,
    base_url: String,
    collection: String,
}

/// Convert the backend-neutral governed filter into Qdrant's filter DSL.
/// Unknown/malformed clauses fail closed instead of silently widening search.
#[cfg(any(feature = "vector-qdrant", test))]
fn qdrant_filter(filter: &Value) -> Result<Value> {
    let object = filter
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("vector filter must be an object"))?;
    let mut must = Vec::with_capacity(object.len());
    for (key, clause) in object {
        let clause = clause
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("vector filter clause '{}' must be an object", key))?;
        if let Some(values) = clause.get("$in").and_then(Value::as_array) {
            must.push(serde_json::json!({
                "key": key,
                "match": { "any": values }
            }));
        } else if let Some(lte) = clause.get("$lte") {
            let number = lte.as_f64().ok_or_else(|| {
                anyhow::anyhow!("vector filter '$lte' for '{}' must be numeric", key)
            })?;
            must.push(serde_json::json!({
                "key": key,
                "range": { "lte": number }
            }));
        } else {
            anyhow::bail!("unsupported vector filter operator for '{}'", key);
        }
    }
    Ok(serde_json::json!({ "must": must }))
}

#[cfg(feature = "vector-qdrant")]
impl QdrantVectorStore {
    pub async fn new(config: &VectorStoreConfig) -> Result<Self> {
        let store = Self {
            client: reqwest::Client::new(),
            base_url: config.qdrant_url.trim_end_matches('/').to_string(),
            collection: config.collection.clone(),
        };
        let collection_url = format!("{}/collections/{}", store.base_url, store.collection);
        let response = store.client.get(&collection_url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            store
                .client
                .put(&collection_url)
                .json(&serde_json::json!({
                    "vectors": { "size": config.dimension, "distance": "Cosine" }
                }))
                .send()
                .await?
                .error_for_status()?;
        } else {
            response.error_for_status()?;
        }
        Ok(store)
    }

    fn points_url(&self, suffix: &str) -> String {
        format!(
            "{}/collections/{}/points{}",
            self.base_url, self.collection, suffix
        )
    }
}

#[cfg(feature = "vector-qdrant")]
#[async_trait]
impl VectorStore for QdrantVectorStore {
    async fn upsert(&self, id: &str, embedding: &[f32], metadata: Value) -> Result<()> {
        self.client
            .put(format!("{}?wait=true", self.points_url("")))
            .json(&serde_json::json!({
                "points": [{ "id": id, "vector": embedding, "payload": metadata }]
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn search(
        &self,
        query: &[f32],
        top_k: usize,
        filter: Option<Value>,
    ) -> Result<Vec<ScoredEntry>> {
        let filter = filter.as_ref().map(qdrant_filter).transpose()?;
        let response: Value = self
            .client
            .post(self.points_url("/search"))
            .json(&serde_json::json!({
                "vector": query,
                "limit": top_k,
                "with_payload": true,
                "filter": filter
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response
            .get("result")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|point| ScoredEntry {
                id: point
                    .get("id")
                    .map(|id| {
                        id.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| id.to_string())
                    })
                    .unwrap_or_default(),
                score: point
                    .get("score")
                    .and_then(Value::as_f64)
                    .unwrap_or_default() as f32,
                metadata: point.get("payload").cloned().unwrap_or(Value::Null),
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.client
            .post(format!("{}?wait=true", self.points_url("/delete")))
            .json(&serde_json::json!({ "points": [id] }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn count(&self) -> Result<usize> {
        let response: Value = self
            .client
            .post(self.points_url("/count"))
            .json(&serde_json::json!({ "exact": true }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response
            .pointer("/result/count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize)
    }
}

// === LanceDB Implementation ===

#[cfg(feature = "vector-lance")]
pub struct LanceVectorStore {
    db: lancedb::connection::Connection,
    table_name: String,
    dimension: usize,
}

#[cfg(feature = "vector-lance")]
impl LanceVectorStore {
    pub async fn new(config: &VectorStoreConfig) -> Result<Self> {
        use lancedb::connection::ConnectBuilder;

        let db_path = std::path::Path::new(&config.lance_path);
        tokio::fs::create_dir_all(db_path.parent().unwrap_or(db_path)).await?;

        let db = lancedb::connect(&config.lance_path).execute().await?;

        Ok(Self {
            db,
            table_name: config.collection.clone(),
            dimension: config.dimension,
        })
    }
}

#[cfg(feature = "vector-lance")]
#[async_trait]
impl VectorStore for LanceVectorStore {
    async fn upsert(&self, _id: &str, _embedding: &[f32], _metadata: Value) -> Result<()> {
        // LanceDB implementation
        // TODO: Implement with Arrow RecordBatch
        anyhow::bail!("LanceDB upsert not yet fully implemented")
    }

    async fn search(
        &self,
        _query: &[f32],
        _top_k: usize,
        _filter: Option<Value>,
    ) -> Result<Vec<ScoredEntry>> {
        anyhow::bail!("LanceDB search not yet fully implemented")
    }

    async fn delete(&self, _id: &str) -> Result<()> {
        anyhow::bail!("LanceDB delete not yet fully implemented")
    }

    async fn count(&self) -> Result<usize> {
        anyhow::bail!("LanceDB count not yet fully implemented")
    }
}

// === Embedder Trait ===

/// Trait for text embedding models.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a text string into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Return the embedding dimension.
    fn dimension(&self) -> usize;
}

/// OpenAI-compatible embedding API.
pub struct OpenAIEmbedder {
    client: reqwest::Client,
    grant: crate::credential::EmbeddingCredentialGrant,
    base_url: String,
    model: String,
    dim: usize,
}

impl OpenAIEmbedder {
    /// Construct an embedder whose credential is explicitly scoped to the
    /// exact provider base URL. Unscoped raw-key construction is intentionally
    /// unsupported.
    pub fn new_scoped(
        grant: crate::credential::EmbeddingCredentialGrant,
        base_url: impl Into<String>,
        model: impl Into<String>,
        dimension: usize,
    ) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let now = epoch_seconds();
        if !grant.approved_for(&base_url, now) {
            anyhow::bail!("embedding credential is not approved for configured endpoint");
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            grant,
            base_url,
            model: model.into(),
            dim: dimension,
        })
    }

    fn authorization_value(&self, now_unix: u64) -> Result<String> {
        let secret = self
            .grant
            .secret_if_approved(&self.base_url, now_unix)
            .ok_or_else(|| anyhow::anyhow!("embedding endpoint credential rejected"))?;
        Ok(format!("Bearer {}", secret))
    }
}

fn epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let authorization = self.authorization_value(epoch_seconds())?;
        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", authorization)
            .json(&serde_json::json!({
                "input": text,
                "model": self.model,
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

        let embedding = resp["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding response"))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// Create a vector store based on configuration.
pub async fn create_vector_store(config: &VectorStoreConfig) -> Result<Box<dyn VectorStore>> {
    match config.backend {
        VectorBackend::Qdrant => {
            #[cfg(feature = "vector-qdrant")]
            {
                let store = QdrantVectorStore::new(config).await?;
                return Ok(Box::new(store));
            }
            #[cfg(not(feature = "vector-qdrant"))]
            anyhow::bail!("Qdrant backend requested but 'vector-qdrant' feature not enabled")
        }
        VectorBackend::Lance => {
            #[cfg(feature = "vector-lance")]
            {
                let store = LanceVectorStore::new(config).await?;
                return Ok(Box::new(store));
            }
            #[cfg(not(feature = "vector-lance"))]
            anyhow::bail!("LanceDB backend requested but 'vector-lance' feature not enabled")
        }
        VectorBackend::Auto => {
            // Try Qdrant first, fall back to LanceDB
            #[cfg(feature = "vector-qdrant")]
            {
                if let Ok(store) = QdrantVectorStore::new(config).await {
                    return Ok(Box::new(store));
                }
            }
            #[cfg(feature = "vector-lance")]
            {
                let store = LanceVectorStore::new(config).await?;
                return Ok(Box::new(store));
            }
            anyhow::bail!("No vector store backend available. Enable 'vector-qdrant' or 'vector-lance' feature.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_store_config_default() {
        let config = VectorStoreConfig::default();
        assert_eq!(config.backend, VectorBackend::Auto);
        assert_eq!(config.dimension, 384);
        assert_eq!(config.collection, "archival_memory");
    }

    #[test]
    fn test_scored_entry_serialization() {
        let entry = ScoredEntry {
            id: "test-1".to_string(),
            score: 0.95,
            metadata: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: ScoredEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test-1");
        assert!((deserialized.score - 0.95).abs() < 0.001);
    }

    #[test]
    fn governed_filter_translates_to_qdrant_must_conditions() {
        let neutral = serde_json::json!({
            "scope_key": { "$in": ["principal:p", "session:s"] },
            "sensitivity_ord": { "$lte": 1 },
            "authority": { "$in": ["UserExplicit", "Observed"] }
        });
        let filter = qdrant_filter(&neutral).unwrap();
        let must = filter["must"].as_array().unwrap();
        assert_eq!(must.len(), 3);
        assert!(must.iter().any(|condition| {
            condition["key"] == "scope_key"
                && condition["match"]["any"] == serde_json::json!(["principal:p", "session:s"])
        }));
        assert!(must.iter().any(|condition| {
            condition["key"] == "sensitivity_ord" && condition["range"]["lte"] == 1.0
        }));
    }

    #[test]
    fn malformed_or_unknown_filter_operator_fails_closed() {
        assert!(qdrant_filter(&serde_json::json!({
            "scope_key": { "$contains": "principal:p" }
        }))
        .is_err());
        assert!(qdrant_filter(&serde_json::json!(["not", "an", "object"])).is_err());
    }

    fn scoped_grant(base_url: &str) -> crate::credential::EmbeddingCredentialGrant {
        crate::credential::EmbeddingCredentialGrant::new(
            "test-principal",
            base_url,
            "test-embedding",
            epoch_seconds() + 3600,
            1,
            "super-secret-key",
        )
    }

    #[test]
    fn openai_embedder_requires_exact_explicit_scope() {
        let base = "https://embedding.example/v1";
        let embedder = OpenAIEmbedder::new_scoped(scoped_grant(base), base, "model", 3).unwrap();
        assert_eq!(
            embedder.authorization_value(epoch_seconds()).unwrap(),
            "Bearer super-secret-key"
        );
        assert!(OpenAIEmbedder::new_scoped(
            scoped_grant(base),
            "https://embedding.example.evil/v1",
            "model",
            3,
        )
        .is_err());
    }

    #[tokio::test]
    async fn openai_embedder_does_not_follow_redirect_with_credential() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination_addr = destination.local_addr().unwrap();
        let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source_addr = source.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = source.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let size = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..size])
                .to_ascii_lowercase()
                .contains("authorization: bearer redirect-secret"));
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{}/stolen\r\nContent-Length: 0\r\n\r\n",
                        destination_addr
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let base = format!("http://{}", source_addr);
        let grant = crate::credential::EmbeddingCredentialGrant::new(
            "test-principal",
            &base,
            "test-embedding",
            epoch_seconds() + 3600,
            1,
            "redirect-secret",
        );
        let embedder = OpenAIEmbedder::new_scoped(grant, base, "model", 3).unwrap();
        assert!(embedder.embed("hello").await.is_err());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), destination.accept())
                .await
                .is_err()
        );
    }
}
