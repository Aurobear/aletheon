//! Durable exact-vector backend for bounded personal memory collections.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{ensure, Context};
use async_trait::async_trait;
use fabric::EmbeddingProvider;
use rusqlite::{params, Connection};

use crate::recall::pipeline::{
    RankedRecallItem, RecallSearchBackend, ScopePredicate, SearchOutcome,
};
use crate::{MemoryScope, RecallItem, RecallRequest};

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedRecord {
    pub item: RecallItem,
    pub embedding: Vec<f32>,
    pub provider_id: String,
    pub model_id: String,
    pub rotation_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorIndexState {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub dimension: Option<usize>,
    pub rotation_generation: Option<u64>,
    pub record_count: usize,
}

#[async_trait]
pub trait VectorIndexWriter: Send + Sync {
    async fn upsert_batch(&self, records: &[EmbeddedRecord]) -> anyhow::Result<()>;
    async fn remove(&self, record_ids: &[String]) -> anyhow::Result<()>;
    async fn state(&self) -> anyhow::Result<VectorIndexState>;
}

pub struct SqliteVectorBackend {
    db: Mutex<Connection>,
    embedding: Arc<dyn EmbeddingProvider>,
    provider_id: String,
    rotation_generation: u64,
}

impl SqliteVectorBackend {
    pub fn open(
        path: &Path,
        embedding: Arc<dyn EmbeddingProvider>,
        provider_id: impl Into<String>,
        rotation_generation: u64,
    ) -> anyhow::Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS memory_vectors (
               record_id TEXT PRIMARY KEY,
               item_json TEXT NOT NULL,
               scope_key TEXT NOT NULL,
               sensitivity_ord INTEGER NOT NULL,
               authority_json TEXT NOT NULL,
               embedding BLOB NOT NULL,
               dimension INTEGER NOT NULL,
               provider_id TEXT NOT NULL,
               model_id TEXT NOT NULL,
               rotation_generation INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS memory_vectors_scope
               ON memory_vectors(scope_key, sensitivity_ord);",
        )?;
        Ok(Self {
            db: Mutex::new(db),
            embedding,
            provider_id: provider_id.into(),
            rotation_generation,
        })
    }

    fn configured_stamp(&self) -> (&str, &str, usize, u64) {
        (
            &self.provider_id,
            self.embedding.model_id(),
            self.embedding.dimension(),
            self.rotation_generation,
        )
    }

    fn state_sync(&self) -> anyhow::Result<VectorIndexState> {
        let db = self.db.lock().expect("vector database lock poisoned");
        let mut statement = db.prepare(
            "SELECT provider_id,model_id,dimension,rotation_generation,COUNT(*)
             FROM memory_vectors GROUP BY provider_id,model_id,dimension,rotation_generation
             ORDER BY COUNT(*) DESC LIMIT 1",
        )?;
        let row = statement
            .query_row([], |row| {
                Ok(VectorIndexState {
                    provider_id: Some(row.get(0)?),
                    model_id: Some(row.get(1)?),
                    dimension: Some(row.get::<_, i64>(2)? as usize),
                    rotation_generation: Some(row.get::<_, i64>(3)? as u64),
                    record_count: row.get::<_, i64>(4)? as usize,
                })
            })
            .optional()?;
        Ok(row.unwrap_or(VectorIndexState {
            provider_id: None,
            model_id: None,
            dimension: None,
            rotation_generation: None,
            record_count: 0,
        }))
    }
}

#[async_trait]
impl VectorIndexWriter for SqliteVectorBackend {
    async fn upsert_batch(&self, records: &[EmbeddedRecord]) -> anyhow::Result<()> {
        let mut db = self.db.lock().expect("vector database lock poisoned");
        let tx = db.transaction()?;
        for record in records {
            ensure!(!record.embedding.is_empty(), "embedding must not be empty");
            ensure!(
                record.embedding.len() == self.embedding.dimension(),
                "embedding dimension mismatch"
            );
            let item_json = serde_json::to_string(&record.item)?;
            let authority_json = serde_json::to_string(&record.item.authority)?;
            tx.execute(
                "INSERT INTO memory_vectors
                 (record_id,item_json,scope_key,sensitivity_ord,authority_json,embedding,
                  dimension,provider_id,model_id,rotation_generation,updated_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,unixepoch('subsec') * 1000)
                 ON CONFLICT(record_id) DO UPDATE SET
                   item_json=excluded.item_json, scope_key=excluded.scope_key,
                   sensitivity_ord=excluded.sensitivity_ord,
                   authority_json=excluded.authority_json, embedding=excluded.embedding,
                   dimension=excluded.dimension, provider_id=excluded.provider_id,
                   model_id=excluded.model_id,
                   rotation_generation=excluded.rotation_generation,
                   updated_at_ms=excluded.updated_at_ms",
                params![
                    record.item.metadata.record_id,
                    item_json,
                    scope_key(&record.item.scope),
                    sensitivity_ord(record.item.metadata.sensitivity),
                    authority_json,
                    encode_vector(&record.embedding),
                    record.embedding.len() as i64,
                    record.provider_id,
                    record.model_id,
                    record.rotation_generation as i64,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn remove(&self, record_ids: &[String]) -> anyhow::Result<()> {
        let mut db = self.db.lock().expect("vector database lock poisoned");
        let tx = db.transaction()?;
        for id in record_ids {
            tx.execute("DELETE FROM memory_vectors WHERE record_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn state(&self) -> anyhow::Result<VectorIndexState> {
        self.state_sync()
    }
}

#[async_trait]
impl RecallSearchBackend for SqliteVectorBackend {
    async fn search(
        &self,
        request: &RecallRequest,
        predicate: &ScopePredicate,
        top_k: usize,
    ) -> anyhow::Result<SearchOutcome> {
        let query = self.embedding.embed(&request.query).await?;
        ensure!(
            query.len() == self.embedding.dimension(),
            "query dimension mismatch"
        );
        let (provider_id, model_id, dimension, generation) = self.configured_stamp();
        let scope_json = serde_json::to_string(&predicate.scope_keys)?;
        let authority_json = serde_json::to_string(&predicate.allowed_authorities)?;
        let db = self.db.lock().expect("vector database lock poisoned");
        let mut statement = db.prepare(
            "SELECT item_json,embedding FROM memory_vectors
             WHERE scope_key IN (SELECT value FROM json_each(?1))
               AND sensitivity_ord <= ?2
               AND authority_json IN (SELECT json_quote(value) FROM json_each(?3))
               AND provider_id=?4 AND model_id=?5 AND dimension=?6
               AND rotation_generation=?7",
        )?;
        let rows = statement.query_map(
            params![
                scope_json,
                predicate.max_sensitivity_ord,
                authority_json,
                provider_id,
                model_id,
                dimension as i64,
                generation as i64,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        let query_norm = l2_norm(&query);
        let mut items = Vec::new();
        for row in rows {
            let (item_json, bytes) = row?;
            let item: RecallItem =
                serde_json::from_str(&item_json).context("decode vector item")?;
            let embedding = decode_vector(&bytes)?;
            items.push(RankedRecallItem {
                item,
                score: cosine_similarity(&query, query_norm, &embedding),
            });
        }
        drop(statement);
        drop(db);
        items.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.item
                        .metadata
                        .record_id
                        .cmp(&right.item.metadata.record_id)
                })
        });
        items.truncate(top_k);
        let state = self.state_sync()?;
        let index_stale = state.record_count > 0
            && (state.provider_id.as_deref() != Some(provider_id)
                || state.model_id.as_deref() != Some(model_id)
                || state.dimension != Some(dimension)
                || state.rotation_generation != Some(generation));
        Ok(SearchOutcome { items, index_stale })
    }
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_vector(bytes: &[u8]) -> anyhow::Result<Vec<f32>> {
    ensure!(bytes.len().is_multiple_of(4), "invalid vector blob");
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte chunk")))
        .collect())
}

fn l2_norm(vector: &[f32]) -> f32 {
    vector.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn cosine_similarity(left: &[f32], left_norm: f32, right: &[f32]) -> f32 {
    if left.len() != right.len() || left_norm == 0.0 {
        return 0.0;
    }
    let right_norm = l2_norm(right);
    if right_norm == 0.0 {
        return 0.0;
    }
    left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>() / (left_norm * right_norm)
}

fn scope_key(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".into(),
        MemoryScope::Principal(id) => format!("principal:{id}"),
        MemoryScope::Workspace(id) => format!("workspace:{id}"),
        MemoryScope::Session(id) => format!("session:{id}"),
        MemoryScope::Goal(id) => format!("goal:{id}"),
        MemoryScope::Agent(id) => format!("agent:{id}"),
        MemoryScope::Task(id) => format!("task:{id}"),
    }
}

fn sensitivity_ord(value: crate::MemorySensitivity) -> u8 {
    match value {
        crate::MemorySensitivity::Public => 0,
        crate::MemorySensitivity::Internal => 1,
        crate::MemorySensitivity::Confidential => 2,
        crate::MemorySensitivity::Restricted => 3,
    }
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    struct TestEmbedding;

    #[async_trait]
    impl EmbeddingProvider for TestEmbedding {
        async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
            Ok(match text {
                "alpha" => vec![1.0, 0.0],
                "beta" => vec![0.0, 1.0],
                _ => vec![0.5, 0.5],
            })
        }
        fn dimension(&self) -> usize {
            2
        }
        fn model_id(&self) -> &str {
            "test-v1"
        }
    }

    fn item(id: &str, content: &str, scope: MemoryScope) -> RecallItem {
        RecallItem {
            kind: crate::MemoryKind::SemanticFact,
            content: content.into(),
            metadata: crate::MemoryMetadata::local(id, id, Utc::now()),
            temporal_state: crate::TemporalState::Current,
            authority: crate::MemoryAuthority::RawExperience,
            scope,
            score: 0.0,
            evidence: None,
        }
    }

    fn predicate(scope: &str) -> ScopePredicate {
        ScopePredicate {
            scope_keys: vec![scope.into()],
            max_sensitivity_ord: 1,
            allowed_authorities: vec![crate::MemoryAuthority::RawExperience],
        }
    }

    #[tokio::test]
    async fn persists_orders_filters_and_removes_vectors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vectors.db");
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(TestEmbedding);
        {
            let backend = SqliteVectorBackend::open(&path, provider.clone(), "test", 1).unwrap();
            backend
                .upsert_batch(&[
                    EmbeddedRecord {
                        item: item("a", "alpha", MemoryScope::Task("allowed".into())),
                        embedding: vec![1.0, 0.0],
                        provider_id: "test".into(),
                        model_id: "test-v1".into(),
                        rotation_generation: 1,
                    },
                    EmbeddedRecord {
                        item: item("b", "beta", MemoryScope::Task("denied".into())),
                        embedding: vec![0.0, 1.0],
                        provider_id: "test".into(),
                        model_id: "test-v1".into(),
                        rotation_generation: 1,
                    },
                ])
                .await
                .unwrap();
        }
        let backend = SqliteVectorBackend::open(&path, provider, "test", 1).unwrap();
        let outcome = backend
            .search(
                &RecallRequest::bounded("session", "alpha"),
                &predicate("task:allowed"),
                10,
            )
            .await
            .unwrap();
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].item.metadata.record_id, "a");
        assert!(!outcome.index_stale);
        backend.remove(&["a".into()]).await.unwrap();
        assert_eq!(backend.state().await.unwrap().record_count, 1);
    }

    #[tokio::test]
    async fn changed_model_marks_existing_index_stale_without_panicking() {
        struct V2;
        #[async_trait]
        impl EmbeddingProvider for V2 {
            async fn embed(&self, _: &str) -> anyhow::Result<Vec<f32>> {
                Ok(vec![1.0; 3])
            }
            fn dimension(&self) -> usize {
                3
            }
            fn model_id(&self) -> &str {
                "test-v2"
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vectors.db");
        let backend = SqliteVectorBackend::open(&path, Arc::new(TestEmbedding), "test", 1).unwrap();
        backend
            .upsert_batch(&[EmbeddedRecord {
                item: item("a", "alpha", MemoryScope::Global),
                embedding: vec![1.0, 0.0],
                provider_id: "test".into(),
                model_id: "test-v1".into(),
                rotation_generation: 1,
            }])
            .await
            .unwrap();
        drop(backend);
        let changed = SqliteVectorBackend::open(&path, Arc::new(V2), "test", 1).unwrap();
        let outcome = changed
            .search(
                &RecallRequest::bounded("s", "alpha"),
                &predicate("global"),
                10,
            )
            .await
            .unwrap();
        assert!(outcome.items.is_empty());
        assert!(outcome.index_stale);
    }
}
