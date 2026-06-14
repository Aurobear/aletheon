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
        }
    }
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
    client: qdrant_client::Qdrant,
    collection: String,
    dimension: usize,
}

#[cfg(feature = "vector-qdrant")]
impl QdrantVectorStore {
    pub async fn new(config: &VectorStoreConfig) -> Result<Self> {
        use qdrant_client::prelude::*;
        use qdrant_client::qdrant::{
            vectors_config::Config, CreateCollection, Distance, VectorParams, VectorsConfig,
        };

        let client = Qdrant::from_url(&config.qdrant_url).build()?;

        // Create collection if it doesn't exist
        let collections = client.list_collections().await?;
        let exists = collections
            .collections
            .iter()
            .any(|c| c.name == config.collection);

        if !exists {
            client
                .create_collection(&CreateCollection {
                    collection_name: config.collection.clone(),
                    vectors_config: Some(VectorsConfig {
                        config: Some(Config::Params(VectorParams {
                            size: config.dimension as u64,
                            distance: Distance::Cosine.into(),
                            ..Default::default()
                        })),
                    }),
                    ..Default::default()
                })
                .await?;
        }

        Ok(Self {
            client,
            collection: config.collection.clone(),
            dimension: config.dimension,
        })
    }
}

#[cfg(feature = "vector-qdrant")]
#[async_trait]
impl VectorStore for QdrantVectorStore {
    async fn upsert(&self, id: &str, embedding: &[f32], metadata: Value) -> Result<()> {
        use qdrant_client::prelude::*;
        use qdrant_client::qdrant::{PointStruct, Vectors};

        let point = PointStruct::new(
            id.to_string(),
            Vectors::from(embedding.to_vec()),
            serde_json::from_value(metadata)?,
        );

        self.client
            .upsert_points(&self.collection, vec![point], None)
            .await?;

        Ok(())
    }

    async fn search(
        &self,
        query: &[f32],
        top_k: usize,
        _filter: Option<Value>,
    ) -> Result<Vec<ScoredEntry>> {
        use qdrant_client::prelude::*;
        use qdrant_client::qdrant::{SearchPoints, WithPayloadSelector};

        let search = SearchPoints {
            collection_name: self.collection.clone(),
            vector: query.to_vec(),
            limit: top_k as u64,
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            ..Default::default()
        };

        let result = self.client.search_points(&search).await?;

        Ok(result
            .result
            .into_iter()
            .map(|point| ScoredEntry {
                id: point.id.map(|id| id.to_string()).unwrap_or_default(),
                score: point.score,
                metadata: serde_json::to_value(point.payload).unwrap_or_default(),
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        use qdrant_client::prelude::*;
        use qdrant_client::qdrant::{points_selector::PointsSelectorOneOf, PointId, PointsSelector};

        self.client
            .delete_points(
                &self.collection,
                &PointsSelector {
                    points_selector_one_of: Some(PointsSelectorOneOf::Points(
                        PointIdsList {
                            ids: vec![PointId::from(id.to_string())],
                        },
                    )),
                },
                None,
            )
            .await?;

        Ok(())
    }

    async fn count(&self) -> Result<usize> {
        let info = self.client.collection_info(&self.collection).await?;
        Ok(info
            .result
            .and_then(|i| i.points_count)
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
    api_key: String,
    base_url: String,
    model: String,
    dim: usize,
}

impl OpenAIEmbedder {
    pub fn new(api_key: String, model: String, dimension: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            model,
            dim: dimension,
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl Embedder for OpenAIEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": text,
                "model": self.model,
            }))
            .send()
            .await?
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
}
