//! EpisodicMemory — what happened, when, what was done, outcome.

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

use crate::schema;

/// Episodic memory backend — stores events, actions, observations.
pub struct EpisodicMemory {
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl EpisodicMemory {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
        }
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard
            .as_ref()
            .expect("EpisodicMemory not initialized — call init() first");
        f(conn)
    }
}

#[async_trait]
impl Subsystem for EpisodicMemory {
    fn name(&self) -> &str {
        "episodic_memory"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Failed to open {}", self.db_path.display()))?;
        schema::init_base_table(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS episodic_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id   TEXT NOT NULL,
                session_id  TEXT NOT NULL DEFAULT '',
                event_type  TEXT NOT NULL DEFAULT '',
                summary     TEXT NOT NULL DEFAULT '',
                raw_content BLOB,
                context     TEXT NOT NULL DEFAULT '{}',
                created_at  TEXT NOT NULL
            );",
        )?;
        self.conn = Mutex::new(Some(conn));
        tracing::info!(path = %self.db_path.display(), "EpisodicMemory initialized");
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
        memory_type: MemoryType::Episodic,
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
impl MemoryBackend for EpisodicMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        self.with_conn(|conn| {
            let id = entry.id;
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;

            conn.execute(
                "INSERT INTO aletheon_memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.to_string(),
                    "episodic",
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
                "INSERT INTO episodic_events (memory_id, session_id, event_type, summary, raw_content, context, created_at)
                 VALUES (?1, '', '', '', ?2, '{}', ?3)",
                params![id.to_string(), entry.content, now],
            )?;

            Ok(MemoryHandle {
                id,
                memory_type: MemoryType::Episodic,
            })
        })
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT m.* FROM aletheon_memory m WHERE m.memory_type = 'episodic'",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref text) = query.text {
                sql += &format!(
                    " AND (CAST(m.content AS TEXT) LIKE ?{idx} OR EXISTS (SELECT 1 FROM episodic_events e WHERE e.memory_id = m.id AND (e.summary LIKE ?{idx} OR CAST(e.raw_content AS TEXT) LIKE ?{idx})))",
                    idx = param_idx
                );
                param_values.push(Box::new(format!("%{}%", text)));
                param_idx += 1;
            }

            if let Some((start, end)) = &query.time_range {
                sql += &format!(
                    " AND m.created_at >= ?{s} AND m.created_at <= ?{e}",
                    s = param_idx,
                    e = param_idx + 1
                );
                param_values.push(Box::new(start.to_rfc3339()));
                param_values.push(Box::new(end.to_rfc3339()));
                param_idx += 2;
            }

            if let Some(ref tags) = query.tags {
                for tag in tags {
                    sql += &format!(" AND m.tags LIKE ?{idx}", idx = param_idx);
                    param_values.push(Box::new(format!("%{}%", tag)));
                    param_idx += 1;
                }
            }

            if let Some(min_imp) = query.min_importance {
                sql += &format!(" AND m.importance >= ?{idx}", idx = param_idx);
                param_values.push(Box::new(min_imp));
                param_idx += 1;
            }

            sql += " ORDER BY m.created_at DESC";

            if query.limit > 0 {
                sql += &format!(" LIMIT ?{idx}", idx = param_idx);
                param_values.push(Box::new(query.limit as i64));
            }

            let mut stmt = conn.prepare(&sql)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let entries = stmt
                .query_map(params_refs.as_slice(), row_to_entry)?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Increment access count for recalled entries
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
            let mut sql = String::from(
                "SELECT * FROM aletheon_memory WHERE memory_type = 'episodic'",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref tags) = filter.tags {
                for tag in tags {
                    sql += &format!(" AND tags LIKE ?{idx}", idx = param_idx);
                    param_values.push(Box::new(format!("%{}%", tag)));
                    param_idx += 1;
                }
            }

            sql += " ORDER BY created_at DESC";

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
                "DELETE FROM episodic_events WHERE memory_id = ?1",
                params![id],
            )?;
            conn.execute("DELETE FROM aletheon_memory WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        self.with_conn(|conn| {
            let before: i64 = conn.query_row(
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'episodic'",
                [],
                |r| r.get(0),
            )?;

            match strategy {
                CompactStrategy::PruneBelowImportance { threshold } => {
                    conn.execute(
                        "DELETE FROM aletheon_memory WHERE memory_type = 'episodic' AND importance < ?1",
                        params![threshold],
                    )?;
                }
                CompactStrategy::KeepTopN { n } => {
                    conn.execute(
                        "DELETE FROM aletheon_memory WHERE memory_type = 'episodic' AND id NOT IN (
                            SELECT id FROM aletheon_memory WHERE memory_type = 'episodic'
                            ORDER BY importance DESC LIMIT ?1
                        )",
                        params![n as i64],
                    )?;
                }
                CompactStrategy::AgeBased {
                    max_age,
                    min_access_count,
                } => {
                    let cutoff = (Utc::now() - max_age).to_rfc3339();
                    conn.execute(
                        "DELETE FROM aletheon_memory WHERE memory_type = 'episodic'
                         AND created_at < ?1 AND access_count < ?2",
                        params![cutoff, min_access_count as i64],
                    )?;
                }
                CompactStrategy::MergeSimilar { .. } => {
                    // No-op for episodic — events are unique by definition
                }
            }

            // Clean up orphaned episodic_events
            conn.execute(
                "DELETE FROM episodic_events WHERE memory_id NOT IN (SELECT id FROM aletheon_memory)",
                [],
            )?;

            let after: i64 = conn.query_row(
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'episodic'",
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
                "SELECT COUNT(*) FROM aletheon_memory WHERE memory_type = 'episodic'",
                [],
                |r| r.get(0),
            )?;
            let total_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM aletheon_memory WHERE memory_type = 'episodic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let oldest: Option<String> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM aletheon_memory WHERE memory_type = 'episodic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let newest: Option<String> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM aletheon_memory WHERE memory_type = 'episodic'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);

            let mut by_type = std::collections::HashMap::new();
            by_type.insert(MemoryType::Episodic, total as usize);

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

    fn setup() -> (tempfile::NamedTempFile, EpisodicMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = EpisodicMemory::new(tmp.path().to_path_buf());
        (tmp, mem)
    }

    async fn init_mem(mem: &mut EpisodicMemory) {
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
            memory_type: MemoryType::Episodic,
            content: content.to_vec(),
            tags: vec!["test".into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 0.7,
            decay_rate: 0.1,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_episodic_store_and_recall() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"hello world");
        let handle = mem.store(entry.clone()).await.unwrap();
        assert_eq!(handle.memory_type, MemoryType::Episodic);

        let query = MemoryQuery {
            text: Some("hello".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, b"hello world");
    }

    #[tokio::test]
    async fn test_episodic_recall_by_time_range() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let mut entry = make_entry(b"timed event");
        entry.created_at = "2026-01-15T12:00:00Z".parse().unwrap();
        mem.store(entry).await.unwrap();

        let query = MemoryQuery {
            time_range: Some((
                "2026-01-01T00:00:00Z".parse().unwrap(),
                "2026-01-31T23:59:59Z".parse().unwrap(),
            )),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);

        let query_outside = MemoryQuery {
            time_range: Some((
                "2026-02-01T00:00:00Z".parse().unwrap(),
                "2026-02-28T23:59:59Z".parse().unwrap(),
            )),
            limit: 10,
            ..Default::default()
        };
        let results_empty = mem.recall(&query_outside).await.unwrap();
        assert!(results_empty.is_empty());
    }

    #[tokio::test]
    async fn test_episodic_forget() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"to forget");
        let handle = mem.store(entry).await.unwrap();
        mem.forget(&handle).await.unwrap();

        let query = MemoryQuery {
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_episodic_compact() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        for i in 0..5 {
            let mut entry = make_entry(format!("event {}", i).as_bytes());
            entry.importance = 0.1 * i as f64;
            mem.store(entry).await.unwrap();
        }

        let result = mem
            .compact(CompactStrategy::PruneBelowImportance { threshold: 0.3 })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 5);
        assert!(result.entries_after < 5);
    }

    #[tokio::test]
    async fn test_episodic_stats() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"a")).await.unwrap();
        mem.store(make_entry(b"bb")).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.total_size_bytes, 3); // 1 + 2
        assert!(stats.oldest_entry.is_some());
    }
}
