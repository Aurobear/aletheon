//! SemanticMemory — knowledge, concepts, facts, with FTS5 keyword search
//! and optional embedding-based vector search.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use base::{
    CompactResult, CompactStrategy, EmbeddingProvider, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext,
    SubsystemHealth, Version,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::activation::{compute_activation, ActivationEntry};
use crate::schema;

// ---------------------------------------------------------------------------
// In-memory vector index — pure cosine similarity, no external deps.
// ---------------------------------------------------------------------------

/// A single entry in the vector index.
struct VectorEntry {
    memory_id: Uuid,
    embedding: Vec<f32>,
}

/// Simple in-memory vector index with brute-force cosine similarity search.
///
/// Suitable for small-to-medium collections (up to ~100k entries).  For
/// larger collections a proper ANN index (HNSW, IVF) can be swapped in
/// later without changing the public interface.
struct VectorIndex {
    entries: RwLock<Vec<VectorEntry>>,
}

impl VectorIndex {
    fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }

    /// Insert or update the embedding for a memory entry.
    fn upsert(&self, memory_id: Uuid, embedding: Vec<f32>) {
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
    fn remove(&self, memory_id: &Uuid) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| &e.memory_id != memory_id);
    }

    /// Search for the top-k most similar entries using cosine similarity.
    /// Returns `(memory_id, similarity_score)` pairs sorted by descending score.
    fn search(&self, query: &[f32], top_k: usize) -> Vec<(Uuid, f32)> {
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
    fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }
}

/// Compute the L2 (Euclidean) norm of a vector.
fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Compute cosine similarity between two vectors, given the pre-computed
/// L2 norm of the first vector.
fn cosine_similarity(a: &[f32], a_norm: f32, b: &[f32]) -> f32 {
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
fn hash_embedding(text: &str, dim: usize) -> Vec<f32> {
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
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    vector_index: VectorIndex,
}

impl SemanticMemory {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
            embedding_provider: None,
            vector_index: VectorIndex::new(),
        }
    }

    /// Create a SemanticMemory with an embedding provider for vector search.
    pub fn with_embedding_provider(
        db_path: PathBuf,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
            embedding_provider: Some(provider),
            vector_index: VectorIndex::new(),
        }
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
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
    ) -> Result<Vec<MemoryEntry>> {
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
                    params![id_str],
                    row_to_entry,
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
    async fn generate_embedding(&self, text: &str) -> Option<Vec<f32>> {
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

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
    let id_str: String = row.get("id")?;
    let tags_str: String = row.get("tags")?;
    let assoc_str: String = row.get("associations")?;
    let created_at_str: String = row.get("created_at")?;

    Ok(MemoryEntry {
        id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::nil()),
        memory_type: MemoryType::Semantic,
        content: row.get("content")?,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        created_at: created_at_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
        access_count: row.get::<_, i64>("access_count")? as u64,
        importance: row.get("importance")?,
        decay_rate: row.get("decay_rate")?,
        associations: serde_json::from_str(&assoc_str).unwrap_or_default(),
    })
}

#[async_trait]
impl MemoryBackend for SemanticMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        let id = entry.id;
        let text_content = String::from_utf8_lossy(&entry.content).to_string();

        // Generate embedding before locking the DB connection (embedding
        // calls may be async / network-bound).
        let embedding = self.generate_embedding(&text_content).await;

        self.with_conn(|conn| {
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;

            let title = text_content
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();

            conn.execute(
                "INSERT INTO memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.to_string(),
                    "semantic",
                    entry.content,
                    tags,
                    now,
                    entry.access_count as i64,
                    entry.importance,
                    entry.decay_rate,
                    assoc,
                ],
            )?;

            conn.execute(
                "INSERT INTO semantic_entries (memory_id, title, category, content)
                 VALUES (?1, ?2, '', ?3)",
                params![id.to_string(), title, text_content],
            )?;

            let rowid: i64 = conn.query_row(
                "SELECT id FROM semantic_entries WHERE memory_id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )?;

            conn.execute(
                "INSERT INTO semantic_fts (rowid, title, content) VALUES (?1, ?2, ?3)",
                params![rowid, title, text_content],
            )?;

            Ok(MemoryHandle {
                id,
                memory_type: MemoryType::Semantic,
            })
        })?;

        // Index the embedding in the vector store (outside the DB lock).
        if let Some(emb) = embedding {
            self.vector_index.upsert(id, emb);
        }

        Ok(MemoryHandle {
            id,
            memory_type: MemoryType::Semantic,
        })
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        // If an embedding vector is provided, prefer vector search.
        if let Some(ref query_embedding) = query.semantic {
            let top_k = if query.limit > 0 { query.limit } else { 10 };
            let entries = self.search_by_embedding(query_embedding, top_k).await?;
            if !entries.is_empty() {
                // Update access counts
                self.with_conn(|conn| {
                    for entry in &entries {
                        conn.execute(
                            "UPDATE memory SET access_count = access_count + 1 WHERE id = ?1",
                            params![entry.id.to_string()],
                        )?;
                    }
                    Ok(())
                })?;
                return Ok(entries);
            }
            // Fall through to FTS if vector search returned nothing.
        }

        self.with_conn(|conn| {
            let mut entries;
            if let Some(ref text) = query.text {
                // FTS path: keep BM25 rank as primary relevance filter.
                // Fetch 2x the limit to give activation re-ranking room.
                let fetch_limit = if query.limit > 0 {
                    query.limit * 2
                } else {
                    0
                };
                let sql = format!(
                    "SELECT m.* FROM memory m
                     INNER JOIN semantic_entries se ON se.memory_id = m.id
                     INNER JOIN semantic_fts fts ON fts.rowid = se.id
                     WHERE semantic_fts MATCH ?1
                     ORDER BY rank
                     {}",
                    if fetch_limit > 0 {
                        format!("LIMIT {}", fetch_limit)
                    } else {
                        String::new()
                    }
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![text], row_to_entry)?;
                entries = rows.collect::<std::result::Result<Vec<_>, _>>()?;

                // Activation-based tiebreaker: for entries with similar activation
                // scores (within 10%), prefer the one that ranked higher in FTS.
                // Stable sort preserves FTS rank order for equal-activation entries.
                let now = Utc::now().timestamp();
                let scores: Vec<f64> = entries
                    .iter()
                    .map(|e| {
                        compute_activation(
                            &ActivationEntry::new(
                                e.importance,
                                e.access_count as i64,
                                e.created_at.timestamp(),
                            ),
                            now,
                        )
                    })
                    .collect();
                let mut indexed: Vec<(usize, &MemoryEntry)> =
                    entries.iter().enumerate().collect();
                indexed.sort_by(|&(i, _), &(j, _)| {
                    let (si, sj) = (scores[i], scores[j]);
                    let max_s = si.max(sj);
                    // If scores are within 10% of each other, keep FTS rank order
                    if max_s > 0.0 && (si - sj).abs() / max_s < 0.1 {
                        std::cmp::Ordering::Equal // stable sort preserves FTS order
                    } else {
                        sj.partial_cmp(&si).unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
                entries = indexed.into_iter().map(|(_, e)| e.clone()).collect();
            } else {
                // Non-FTS path: activation-based sorting
                let mut sql =
                    String::from("SELECT * FROM memory WHERE memory_type = 'semantic'");
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                let mut param_idx = 1;

                if let Some(ref tags) = query.tags {
                    for tag in tags {
                        sql += &format!(" AND tags LIKE ?{idx}", idx = param_idx);
                        param_values.push(Box::new(format!("%{}%", tag)));
                        param_idx += 1;
                    }
                }

                if let Some(min_imp) = query.min_importance {
                    sql += &format!(" AND importance >= ?{idx}", idx = param_idx);
                    param_values.push(Box::new(min_imp));
                    param_idx += 1;
                }

                // Fetch without ORDER BY — activation sort happens in Rust.
                // If a limit is set, fetch 2x to give re-ranking room.
                if query.limit > 0 {
                    sql += &format!(" LIMIT ?{idx}", idx = param_idx);
                    param_values.push(Box::new((query.limit as i64) * 2));
                }

                let mut stmt = conn.prepare(&sql)?;
                let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(params_refs.as_slice(), row_to_entry)?;
                entries = rows.collect::<std::result::Result<Vec<_>, _>>()?;

                let now = Utc::now().timestamp();
                entries.sort_by(|a, b| {
                    let sa = compute_activation(
                        &ActivationEntry::new(
                            a.importance,
                            a.access_count as i64,
                            a.created_at.timestamp(),
                        ),
                        now,
                    );
                    let sb = compute_activation(
                        &ActivationEntry::new(
                            b.importance,
                            b.access_count as i64,
                            b.created_at.timestamp(),
                        ),
                        now,
                    );
                    sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                });
            }

            if query.limit > 0 {
                entries.truncate(query.limit);
            }

            for entry in &entries {
                conn.execute(
                    "UPDATE memory SET access_count = access_count + 1 WHERE id = ?1",
                    params![entry.id.to_string()],
                )?;
            }

            Ok(entries)
        })
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql =
                String::from("SELECT * FROM memory WHERE memory_type = 'semantic'");
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref tags) = filter.tags {
                for tag in tags {
                    sql += &format!(" AND tags LIKE ?{idx}", idx = param_idx);
                    param_values.push(Box::new(format!("%{}%", tag)));
                    param_idx += 1;
                }
            }

            sql += " ORDER BY importance DESC";

            if filter.limit > 0 {
                sql += &format!(" LIMIT ?{idx}", idx = param_idx);
                param_values.push(Box::new(filter.limit as i64));
            }

            let mut stmt = conn.prepare(&sql)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let entries = stmt
                .query_map(params_refs.as_slice(), row_to_entry)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(entries)
        })
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        // Remove from vector index
        self.vector_index.remove(&handle.id);

        self.with_conn(|conn| {
            let id = handle.id.to_string();
            conn.execute(
                "DELETE FROM semantic_fts WHERE rowid IN (
                    SELECT id FROM semantic_entries WHERE memory_id = ?1
                )",
                params![id],
            )?;
            conn.execute(
                "DELETE FROM semantic_entries WHERE memory_id = ?1",
                params![id],
            )?;
            conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        self.with_conn(|conn| {
            let before: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'semantic'",
                [],
                |r| r.get(0),
            )?;

            // Collect IDs to remove so we can also clean the vector index.
            let ids_to_remove: Vec<String> = match strategy {
                CompactStrategy::PruneBelowImportance { threshold } => {
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM memory WHERE memory_type = 'semantic' AND importance < ?1",
                        )?
                        .query_map(params![threshold], |r| r.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    for id in &ids {
                        conn.execute(
                            "DELETE FROM semantic_fts WHERE rowid IN (SELECT id FROM semantic_entries WHERE memory_id = ?1)",
                            params![id],
                        )?;
                        conn.execute(
                            "DELETE FROM semantic_entries WHERE memory_id = ?1",
                            params![id],
                        )?;
                    }
                    conn.execute(
                        "DELETE FROM memory WHERE memory_type = 'semantic' AND importance < ?1",
                        params![threshold],
                    )?;
                    ids
                }
                CompactStrategy::KeepTopN { n } => {
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM memory WHERE memory_type = 'semantic'
                             ORDER BY importance DESC LIMIT -1 OFFSET ?1",
                        )?
                        .query_map(params![n as i64], |r| r.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    for id in &ids {
                        conn.execute(
                            "DELETE FROM semantic_fts WHERE rowid IN (SELECT id FROM semantic_entries WHERE memory_id = ?1)",
                            params![id],
                        )?;
                        conn.execute(
                            "DELETE FROM semantic_entries WHERE memory_id = ?1",
                            params![id],
                        )?;
                        conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
                    }
                    ids
                }
                CompactStrategy::MergeSimilar { .. } => {
                    let duplicates: Vec<(String, String)> = conn
                        .prepare(
                            "SELECT se.memory_id, se.title FROM semantic_entries se
                             INNER JOIN memory m ON m.id = se.memory_id
                             WHERE m.memory_type = 'semantic'
                             GROUP BY se.title HAVING COUNT(*) > 1",
                        )?
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    let mut removed_ids = Vec::new();
                    for (_memory_id, title) in &duplicates {
                        let ids_to_remove: Vec<String> = conn
                            .prepare(
                                "SELECT se.memory_id FROM semantic_entries se
                                 INNER JOIN memory m ON m.id = se.memory_id
                                 WHERE se.title = ?1 AND m.memory_type = 'semantic'
                                 ORDER BY m.importance DESC LIMIT -1 OFFSET 1",
                            )?
                            .query_map(params![title], |r| r.get(0))?
                            .collect::<std::result::Result<Vec<_>, _>>()?;

                        for remove_id in &ids_to_remove {
                            conn.execute(
                                "DELETE FROM semantic_fts WHERE rowid IN (SELECT id FROM semantic_entries WHERE memory_id = ?1)",
                                params![remove_id],
                            )?;
                            conn.execute(
                                "DELETE FROM semantic_entries WHERE memory_id = ?1",
                                params![remove_id],
                            )?;
                            conn.execute(
                                "DELETE FROM memory WHERE id = ?1",
                                params![remove_id],
                            )?;
                        }
                        removed_ids.extend(ids_to_remove);
                    }
                    removed_ids
                }
                CompactStrategy::AgeBased {
                    max_age,
                    min_access_count,
                } => {
                    let cutoff = (Utc::now() - max_age).to_rfc3339();
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM memory WHERE memory_type = 'semantic'
                             AND created_at < ?1 AND access_count < ?2",
                        )?
                        .query_map(params![cutoff, min_access_count as i64], |r| r.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    for id in &ids {
                        conn.execute(
                            "DELETE FROM semantic_fts WHERE rowid IN (SELECT id FROM semantic_entries WHERE memory_id = ?1)",
                            params![id],
                        )?;
                        conn.execute(
                            "DELETE FROM semantic_entries WHERE memory_id = ?1",
                            params![id],
                        )?;
                        conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
                    }
                    ids
                }
            };

            // Remove from vector index
            for id_str in &ids_to_remove {
                if let Ok(uuid) = Uuid::parse_str(id_str) {
                    self.vector_index.remove(&uuid);
                }
            }

            let after: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'semantic'",
                [],
                |r| r.get(0),
            )?;

            Ok(CompactResult {
                entries_before: before as usize,
                entries_after: after as usize,
                entries_removed: (before - after) as usize,
                entries_merged: 0,
            })
        })
    }

    async fn stats(&self) -> Result<MemoryStats> {
        self.with_conn(|conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'semantic'",
                [],
                |r| r.get(0),
            )?;
            let total_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM memory WHERE memory_type = 'semantic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let oldest: Option<String> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM memory WHERE memory_type = 'semantic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let newest: Option<String> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM memory WHERE memory_type = 'semantic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);

            let mut by_type = std::collections::HashMap::new();
            by_type.insert(MemoryType::Semantic, total as usize);

            Ok(MemoryStats {
                total_entries: total as usize,
                by_type,
                total_size_bytes: total_size as u64,
                oldest_entry: oldest.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                newest_entry: newest.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::NamedTempFile, SemanticMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = SemanticMemory::new(tmp.path().to_path_buf());
        (tmp, mem)
    }

    fn setup_with_embedding() -> (tempfile::NamedTempFile, SemanticMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let provider = Arc::new(HashEmbeddingProvider::new(32));
        let mem = SemanticMemory::with_embedding_provider(tmp.path().to_path_buf(), provider);
        (tmp, mem)
    }

    async fn init_mem(mem: &mut SemanticMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
        };
        mem.init(&ctx).await.unwrap();
    }

    fn make_entry(content: &[u8]) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            content: content.to_vec(),
            tags: vec!["knowledge".into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 0.8,
            decay_rate: 0.0,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_semantic_store_and_fts_search() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"Rust is a systems programming language"))
            .await
            .unwrap();
        mem.store(make_entry(b"Python is a scripting language"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("systems programming".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(String::from_utf8_lossy(&results[0].content).contains("Rust"));
    }

    #[tokio::test]
    async fn test_semantic_search_ranking() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"memory management in Rust is critical"))
            .await
            .unwrap();
        mem.store(make_entry(b"Rust uses ownership for resource management"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("memory".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
        assert!(String::from_utf8_lossy(&results[0].content).contains("memory management"));
    }

    #[tokio::test]
    async fn test_semantic_merge_similar() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let mut e1 = make_entry(b"Same Title\nVersion A");
        e1.importance = 0.9;
        let mut e2 = make_entry(b"Same Title\nVersion B");
        e2.importance = 0.5;
        mem.store(e1).await.unwrap();
        mem.store(e2).await.unwrap();

        let result = mem
            .compact(CompactStrategy::MergeSimilar {
                similarity_threshold: 0.9,
            })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 2);
        assert_eq!(result.entries_after, 1);
    }

    #[tokio::test]
    async fn test_semantic_forget() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"forgettable knowledge");
        let handle = mem.store(entry).await.unwrap();
        mem.forget(&handle).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    // --- Vector search tests ---

    #[tokio::test]
    async fn test_vector_index_cosine_similarity() {
        let index = VectorIndex::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        // Two similar vectors and one different
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.9, 0.1, 0.0]; // similar to v1
        let v3 = vec![0.0, 0.0, 1.0]; // orthogonal to v1

        index.upsert(id1, v1.clone());
        index.upsert(id2, v2);
        index.upsert(id3, v3);

        let results = index.search(&v1, 3);
        assert_eq!(results.len(), 3);
        // id1 should be first (exact match, score ~1.0)
        assert_eq!(results[0].0, id1);
        assert!((results[0].1 - 1.0).abs() < 0.001);
        // id2 should be second (similar)
        assert_eq!(results[1].0, id2);
        assert!(results[1].1 > 0.8);
        // id3 should be last (orthogonal, score ~0)
        assert_eq!(results[2].0, id3);
        assert!(results[2].1 < 0.1);
    }

    #[tokio::test]
    async fn test_vector_index_top_k() {
        let index = VectorIndex::new();
        let query = vec![1.0, 0.0, 0.0];

        for _ in 0..10 {
            index.upsert(Uuid::new_v4(), vec![0.5, 0.5, 0.0]);
        }

        let results = index.search(&query, 3);
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_vector_index_upsert_updates() {
        let index = VectorIndex::new();
        let id = Uuid::new_v4();

        index.upsert(id, vec![1.0, 0.0, 0.0]);
        assert_eq!(index.len(), 1);

        // Upsert same ID with different embedding
        index.upsert(id, vec![0.0, 1.0, 0.0]);
        assert_eq!(index.len(), 1);

        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert!((results[0].1 - 1.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_vector_index_remove() {
        let index = VectorIndex::new();
        let id = Uuid::new_v4();
        index.upsert(id, vec![1.0, 0.0]);
        assert_eq!(index.len(), 1);

        index.remove(&id);
        assert_eq!(index.len(), 0);
    }

    #[tokio::test]
    async fn test_hash_embedding_deterministic() {
        let provider = HashEmbeddingProvider::new(32);
        let e1 = provider.embed("hello world").await.unwrap();
        let e2 = provider.embed("hello world").await.unwrap();
        assert_eq!(e1, e2);
        assert_eq!(e1.len(), 32);
    }

    #[tokio::test]
    async fn test_hash_embedding_different_texts() {
        let provider = HashEmbeddingProvider::new(32);
        let e1 = provider.embed("hello").await.unwrap();
        let e2 = provider.embed("world").await.unwrap();
        // Different texts should produce different embeddings
        assert_ne!(e1, e2);
    }

    #[tokio::test]
    async fn test_semantic_store_and_vector_search() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        let e1 = make_entry(b"Rust is a systems programming language");
        let e2 = make_entry(b"Python is a scripting language");
        let id1 = e1.id;
        mem.store(e1).await.unwrap();
        mem.store(e2).await.unwrap();

        // Direct vector search should find the stored entries
        let query_text = "Rust is a systems programming language";
        let provider = HashEmbeddingProvider::new(32);
        let query_emb = provider.embed(query_text).await.unwrap();

        let results = mem.search_by_embedding(&query_emb, 10).await.unwrap();
        assert!(!results.is_empty());
        // The exact match should be first
        assert_eq!(results[0].id, id1);
    }

    #[tokio::test]
    async fn test_recall_with_semantic_field() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"quantum computing basics"))
            .await
            .unwrap();
        mem.store(make_entry(b"classical algorithms overview"))
            .await
            .unwrap();

        // Use recall with semantic field set
        let provider = HashEmbeddingProvider::new(32);
        let query_emb = provider.embed("quantum computing basics").await.unwrap();

        let query = MemoryQuery {
            semantic: Some(query_emb),
            limit: 5,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
        assert!(String::from_utf8_lossy(&results[0].content).contains("quantum"));
    }

    #[tokio::test]
    async fn test_recall_semantic_fallback_to_fts() {
        // Without embedding provider, semantic field is ignored, falls back to FTS
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"Rust memory safety guarantees"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("memory".into()),
            semantic: Some(vec![1.0, 0.0]), // will be ignored (no provider)
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_forget_removes_from_vector_index() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        let entry = make_entry(b"forgettable with embedding");
        let handle = mem.store(entry).await.unwrap();

        // Verify vector index has the entry
        let results = mem
            .search_by_embedding(&hash_embedding("forgettable with embedding", 32), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        // Forget
        mem.forget(&handle).await.unwrap();

        // Verify vector index no longer has the entry
        let results = mem
            .search_by_embedding(&hash_embedding("forgettable with embedding", 32), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_cosine_similarity_basic() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let a_norm = l2_norm(&a);
        assert!((cosine_similarity(&a, a_norm, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, a_norm, &c).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert_eq!(cosine_similarity(&a, 0.0, &b), 0.0);
    }

    #[test]
    fn test_l2_norm() {
        assert!((l2_norm(&[3.0, 4.0]) - 5.0).abs() < 0.001);
        assert_eq!(l2_norm(&[0.0, 0.0]), 0.0);
    }
}
