//! SemanticMemory — knowledge, concepts, facts, with FTS5 keyword search.

use std::path::PathBuf;
use std::sync::Mutex;

use aletheon_abi::{
    CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter, MemoryHandle,
    MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext, SubsystemHealth, Version,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::activation::{compute_activation, ActivationEntry};
use crate::schema;

pub struct SemanticMemory {
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl SemanticMemory {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
        }
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard.as_ref().expect("SemanticMemory not initialized");
        f(conn)
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
        self.with_conn(|conn| {
            let id = entry.id;
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;
            let text_content = String::from_utf8_lossy(&entry.content).to_string();

            let title = text_content
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();

            conn.execute(
                "INSERT INTO aletheon_memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
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
        })
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
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
                    "SELECT m.* FROM aletheon_memory m
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
                    String::from("SELECT * FROM aletheon_memory WHERE memory_type = 'semantic'");
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
                    "UPDATE aletheon_memory SET access_count = access_count + 1 WHERE id = ?1",
                    params![entry.id.to_string()],
                )?;
            }

            Ok(entries)
        })
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql =
                String::from("SELECT * FROM aletheon_memory WHERE memory_type = 'semantic'");
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
            conn.execute("DELETE FROM aletheon_memory WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        self.with_conn(|conn| {
            let before: i64 = conn.query_row(
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'semantic'",
                [],
                |r| r.get(0),
            )?;

            match strategy {
                CompactStrategy::PruneBelowImportance { threshold } => {
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM aletheon_memory WHERE memory_type = 'semantic' AND importance < ?1",
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
                        "DELETE FROM aletheon_memory WHERE memory_type = 'semantic' AND importance < ?1",
                        params![threshold],
                    )?;
                }
                CompactStrategy::KeepTopN { n } => {
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM aletheon_memory WHERE memory_type = 'semantic'
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
                        conn.execute("DELETE FROM aletheon_memory WHERE id = ?1", params![id])?;
                    }
                }
                CompactStrategy::MergeSimilar { .. } => {
                    let duplicates: Vec<(String, String)> = conn
                        .prepare(
                            "SELECT se.memory_id, se.title FROM semantic_entries se
                             INNER JOIN aletheon_memory m ON m.id = se.memory_id
                             WHERE m.memory_type = 'semantic'
                             GROUP BY se.title HAVING COUNT(*) > 1",
                        )?
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    for (_memory_id, title) in &duplicates {
                        let ids_to_remove: Vec<String> = conn
                            .prepare(
                                "SELECT se.memory_id FROM semantic_entries se
                                 INNER JOIN aletheon_memory m ON m.id = se.memory_id
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
                                "DELETE FROM aletheon_memory WHERE id = ?1",
                                params![remove_id],
                            )?;
                        }
                    }
                }
                CompactStrategy::AgeBased {
                    max_age,
                    min_access_count,
                } => {
                    let cutoff = (Utc::now() - max_age).to_rfc3339();
                    let ids: Vec<String> = conn
                        .prepare(
                            "SELECT id FROM aletheon_memory WHERE memory_type = 'semantic'
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
                        conn.execute("DELETE FROM aletheon_memory WHERE id = ?1", params![id])?;
                    }
                }
            }

            let after: i64 = conn.query_row(
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'semantic'",
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
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'semantic'",
                [],
                |r| r.get(0),
            )?;
            let total_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM aletheon_memory WHERE memory_type = 'semantic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let oldest: Option<String> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM aletheon_memory WHERE memory_type = 'semantic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let newest: Option<String> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM aletheon_memory WHERE memory_type = 'semantic'",
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
}
