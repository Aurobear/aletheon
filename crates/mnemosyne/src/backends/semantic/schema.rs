//! SemanticMemory schema, vector index, embedding provider, and Subsystem lifecycle.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::{EmbeddingProvider, Subsystem, SubsystemContext, SubsystemHealth, Version};
use rusqlite::Connection;
use uuid::Uuid;

use crate::ops::schema;

// ---------------------------------------------------------------------------
// In-memory vector index — pure cosine similarity, no external deps.
// ---------------------------------------------------------------------------

/// A single entry in the vector index.
pub(super) struct VectorEntry {
    memory_id: Uuid,
    embedding: Vec<f32>,
}

/// Simple in-memory vector index with brute-force cosine similarity search.
///
/// Suitable for small-to-medium collections (up to ~100k entries).  For
/// larger collections a proper ANN index (HNSW, IVF) can be swapped in
/// later without changing the public interface.
pub(super) struct VectorIndex {
    entries: RwLock<Vec<VectorEntry>>,
}

impl VectorIndex {
    pub(super) fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }

    /// Insert or update the embedding for a memory entry.
    pub(super) fn upsert(&self, memory_id: Uuid, embedding: Vec<f32>) {
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.iter_mut().find(|e| e.memory_id == memory_id) {
            entry.embedding = embedding;
        } else {
            entries.push(VectorEntry {
                memory_id,
                embedding,
            });
        }
    }

    /// Remove an entry by memory id.
    pub(super) fn remove(&self, memory_id: &Uuid) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| &e.memory_id != memory_id);
    }

    /// Search for the top-k most similar entries using cosine similarity.
    /// Returns `(memory_id, similarity_score)` pairs sorted by descending score.
    pub(super) fn search(&self, query: &[f32], top_k: usize) -> Vec<(Uuid, f32)> {
        let entries = self.entries.read().unwrap();
        let query_norm = l2_norm(query);
        if query_norm == 0.0 {
            return Vec::new();
        }

        let mut scored: Vec<(Uuid, f32)> = entries
            .iter()
            .map(|e| {
                let sim = cosine_similarity(query, query_norm, &e.embedding);
                (e.memory_id, sim)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    #[allow(dead_code)]
    pub(super) fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }
}

/// Compute the L2 (Euclidean) norm of a vector.
pub(super) fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Compute cosine similarity between two vectors, given the pre-computed
/// L2 norm of the first vector.
pub(super) fn cosine_similarity(a: &[f32], a_norm: f32, b: &[f32]) -> f32 {
    if a.len() != b.len() || a_norm == 0.0 {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let b_norm = l2_norm(b);
    if b_norm == 0.0 {
        0.0
    } else {
        dot / (a_norm * b_norm)
    }
}

// ---------------------------------------------------------------------------
// Hash-based fallback embedding provider — not semantically meaningful,
// but allows the vector pipeline to be exercised without a real model.
// ---------------------------------------------------------------------------

/// A deterministic hash-based embedding provider.
///
/// Produces a fixed-dimension vector from a text string using a simple
/// hash function.  The vectors are NOT semantically meaningful — they
/// exist solely to exercise the VectorIndex plumbing in environments
/// where no real embedding model is available.
pub struct HashEmbeddingProvider {
    dimension: usize,
}

impl HashEmbeddingProvider {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl EmbeddingProvider for HashEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hash_embedding(text, self.dimension))
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Produce a deterministic float vector from text using a simple hash.
///
/// Each byte of the input seeds a position in the output vector via
/// modular indexing.  The result is L2-normalised so cosine similarity
/// is well-defined.
pub(super) fn hash_embedding(text: &str, dim: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dim];
    for (i, byte) in text.bytes().enumerate() {
        let idx = (i.wrapping_mul(31).wrapping_add(byte as usize)) % dim;
        vec[idx] += 1.0;
    }
    // Normalise
    let norm = l2_norm(&vec);
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

// ---------------------------------------------------------------------------
// SemanticMemory
// ---------------------------------------------------------------------------

pub struct SemanticMemory {
    pub(super) db_path: PathBuf,
    pub(super) conn: Mutex<Option<Connection>>,
    pub(super) embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub(super) vector_index: VectorIndex,
    pub(super) clock: Arc<dyn fabric::Clock>,
}

impl SemanticMemory {
    pub fn new(db_path: PathBuf, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
            embedding_provider: None,
            vector_index: VectorIndex::new(),
            clock,
        }
    }

    /// Create a SemanticMemory with an embedding provider for vector search.
    pub fn with_embedding_provider(
        db_path: PathBuf,
        provider: Arc<dyn EmbeddingProvider>,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
            embedding_provider: Some(provider),
            vector_index: VectorIndex::new(),
            clock,
        }
    }

    pub(super) fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard.as_ref().expect("SemanticMemory not initialized");
        f(conn)
    }

    /// Search by embedding vector.  Returns memory entries sorted by
    /// descending cosine similarity.
    pub async fn search_by_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<fabric::MemoryEntry>> {
        let scored = self.vector_index.search(query_embedding, top_k);
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut entries = Vec::new();
            for (memory_id, _score) in &scored {
                let id_str = memory_id.to_string();
                let result = conn.query_row(
                    "SELECT * FROM memory WHERE id = ?1",
                    rusqlite::params![id_str],
                    |row| super::query::row_to_entry(row, &self.clock),
                );
                if let Ok(entry) = result {
                    entries.push(entry);
                }
            }
            Ok(entries)
        })
    }

    /// Generate an embedding for the given text using the configured provider.
    /// Returns None if no embedding provider is configured.
    pub(super) async fn generate_embedding(&self, text: &str) -> Option<Vec<f32>> {
        if let Some(ref provider) = self.embedding_provider {
            match provider.embed(text).await {
                Ok(embedding) => Some(embedding),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to generate embedding, skipping vector index");
                    None
                }
            }
        } else {
            None
        }
    }
}

#[async_trait]
impl Subsystem for SemanticMemory {
    fn name(&self) -> &str {
        "semantic_memory"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Failed to open {}", self.db_path.display()))?;
        schema::init_base_table(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS semantic_entries (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id   TEXT NOT NULL,
                title       TEXT NOT NULL DEFAULT '',
                category    TEXT NOT NULL DEFAULT '',
                content     TEXT NOT NULL DEFAULT ''
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS semantic_fts USING fts5(
                title, content, content_rowid='id', tokenize='porter unicode61'
            );",
        )?;
        self.conn = Mutex::new(Some(conn));
        tracing::info!(path = %self.db_path.display(), "SemanticMemory initialized");
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        let guard = self.conn.lock().unwrap();
        if guard.is_some() {
            SubsystemHealth::Healthy
        } else {
            SubsystemHealth::Degraded {
                reason: "not initialized".into(),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        let mut guard = self.conn.lock().unwrap();
        *guard = None;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}
