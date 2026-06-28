use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::vector_store::{VectorStore, Embedder};

/// Entry in Archival Memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalEntry {
    pub id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// L3 Archival Memory trait -- vector database for long-term knowledge.
/// Phase 2: in-memory stub. Phase 3+: LanceDB or Qdrant implementation.
#[async_trait]
pub trait ArchivalMemory: Send + Sync {
    /// Insert a new entry with optional scope tag in metadata.
    async fn insert(&mut self, content: &str, metadata: serde_json::Value) -> anyhow::Result<String>;

    /// Search by semantic similarity.
    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ArchivalEntry>>;

    /// Search by semantic similarity within a specific scope.
    /// Scope is matched against `metadata["scope"]`.
    async fn search_in_scope(&self, query: &str, limit: usize, scope: &str) -> anyhow::Result<Vec<ArchivalEntry>>;

    /// Get entry by ID.
    async fn get(&self, id: &str) -> anyhow::Result<Option<ArchivalEntry>>;

    /// Delete entry by ID.
    async fn delete(&mut self, id: &str) -> anyhow::Result<bool>;

    /// Count total entries.
    async fn count(&self) -> anyhow::Result<usize>;
}

/// In-memory stub implementation for Phase 2.
pub struct InMemoryArchival {
    entries: Vec<ArchivalEntry>,
    next_id: u64,
}

impl InMemoryArchival {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
        }
    }
}

impl Default for InMemoryArchival {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ArchivalMemory for InMemoryArchival {
    async fn insert(&mut self, content: &str, metadata: serde_json::Value) -> anyhow::Result<String> {
        self.next_id += 1;
        let id = format!("arch_{}", self.next_id);
        self.entries.push(ArchivalEntry {
            id: id.clone(),
            content: content.to_string(),
            embedding: None,
            metadata,
            created_at: chrono::Utc::now(),
        });
        Ok(id)
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ArchivalEntry>> {
        // Simple keyword search for stub; vector search in Phase 3
        let query_lower = query.to_lowercase();
        let mut results: Vec<&ArchivalEntry> = self.entries
            .iter()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .collect();
        results.truncate(limit);
        Ok(results.into_iter().cloned().collect())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<ArchivalEntry>> {
        Ok(self.entries.iter().find(|e| e.id == id).cloned())
    }

    async fn delete(&mut self, id: &str) -> anyhow::Result<bool> {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        Ok(self.entries.len() < len_before)
    }

    async fn search_in_scope(&self, query: &str, limit: usize, scope: &str) -> anyhow::Result<Vec<ArchivalEntry>> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<&ArchivalEntry> = self.entries
            .iter()
            .filter(|e| {
                e.content.to_lowercase().contains(&query_lower)
                    && e.metadata
                        .get("scope")
                        .and_then(|s| s.as_str())
                        .map(|s| s == scope)
                        .unwrap_or(false)
            })
            .collect();
        results.truncate(limit);
        Ok(results.into_iter().cloned().collect())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.len())
    }
}

/// Vector-backed archival memory using a VectorStore for semantic search.
pub struct VectorArchival {
    store: Box<dyn VectorStore>,
    embedder: Box<dyn Embedder>,
}

impl VectorArchival {
    pub fn new(store: Box<dyn VectorStore>, embedder: Box<dyn Embedder>) -> Self {
        Self { store, embedder }
    }
}

#[async_trait]
impl ArchivalMemory for VectorArchival {
    async fn insert(&mut self, content: &str, metadata: serde_json::Value) -> anyhow::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let embedding = self.embedder.embed(content).await?;
        self.store.upsert(&id, &embedding, metadata).await?;
        Ok(id)
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ArchivalEntry>> {
        let embedding = self.embedder.embed(query).await?;
        let scored = self.store.search(&embedding, limit, None).await?;

        Ok(scored
            .into_iter()
            .map(|s| ArchivalEntry {
                id: s.id,
                content: s.metadata["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                embedding: None,
                metadata: s.metadata,
                created_at: chrono::Utc::now(),
            })
            .collect())
    }

    async fn search_in_scope(&self, query: &str, limit: usize, scope: &str) -> anyhow::Result<Vec<ArchivalEntry>> {
        let embedding = self.embedder.embed(query).await?;
        let scored = self.store.search(&embedding, limit * 2, None).await?;

        let results: Vec<ArchivalEntry> = scored
            .into_iter()
            .filter(|s| {
                s.metadata
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(|v| v == scope)
                    .unwrap_or(false)
            })
            .take(limit)
            .map(|s| ArchivalEntry {
                id: s.id,
                content: s.metadata["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                embedding: None,
                metadata: s.metadata,
                created_at: chrono::Utc::now(),
            })
            .collect();

        Ok(results)
    }

    async fn get(&self, _id: &str) -> anyhow::Result<Option<ArchivalEntry>> {
        // VectorStore doesn't have a direct get by ID; search would be needed
        // For now, return None (implement with metadata filter if needed)
        Ok(None)
    }

    async fn delete(&mut self, id: &str) -> anyhow::Result<bool> {
        self.store.delete(id).await?;
        Ok(true)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        self.store.count().await
    }
}
