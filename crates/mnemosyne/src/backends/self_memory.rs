//! SelfMemory — identity changes, lineage graph, boundary decisions.
//!
//! Identity history is permanent: `compact()` is a no-op.
//! `forget()` requires the entry to be approved (approved != 0).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabric::{
    wall_to_datetime, CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext,
    SubsystemHealth, Version, WallTime,
};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::ops::activation::{compute_activation, ActivationEntry};
use crate::ops::schema;

pub struct SelfMemory {
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
    clock: Arc<dyn fabric::Clock>,
}

impl SelfMemory {
    pub fn new(db_path: PathBuf, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
            clock,
        }
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard.as_ref().expect("SelfMemory not initialized");
        f(conn)
    }
}

#[async_trait]
impl Subsystem for SelfMemory {
    fn name(&self) -> &str {
        "self_memory"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Failed to open {}", self.db_path.display()))?;
        schema::init_base_table(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS self_entries (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id    TEXT NOT NULL,
                change_type  TEXT NOT NULL DEFAULT '',
                description  TEXT NOT NULL DEFAULT '',
                before_state TEXT NOT NULL DEFAULT '',
                after_state  TEXT NOT NULL DEFAULT '',
                reason       TEXT NOT NULL DEFAULT '',
                approved     INTEGER NOT NULL DEFAULT 0,
                parent_id    TEXT
            );",
        )?;
        self.conn = Mutex::new(Some(conn));
        tracing::info!(path = %self.db_path.display(), "SelfMemory initialized");
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

fn row_to_entry(
    row: &rusqlite::Row,
    clock: &Arc<dyn fabric::Clock>,
) -> rusqlite::Result<MemoryEntry> {
    let id_str: String = row.get("id")?;
    let tags_str: String = row.get("tags")?;
    let assoc_str: String = row.get("associations")?;
    let created_at_str: String = row.get("created_at")?;

    Ok(MemoryEntry {
        id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::nil()),
        memory_type: MemoryType::SelfMemory,
        content: row.get("content")?,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        created_at: created_at_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| wall_to_datetime(clock.wall_now())),
        access_count: row.get::<_, i64>("access_count")? as u64,
        importance: row.get("importance")?,
        decay_rate: row.get("decay_rate")?,
        associations: serde_json::from_str(&assoc_str).unwrap_or_default(),
    })
}

#[async_trait]
impl MemoryBackend for SelfMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        self.with_conn(|conn| {
            let id = entry.id;
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;

            // Validate parent_id if present in associations
            if let Some(parent_uuid) = entry.associations.first() {
                let parent_exists: bool = conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM memory WHERE id = ?1 AND memory_type = 'self'",
                        params![parent_uuid.to_string()],
                        |r| r.get(0),
                    )
                    .unwrap_or(false);
                if !parent_exists {
                    bail!(
                        "parent_id {} does not exist in self_memory",
                        parent_uuid
                    );
                }
            }

            conn.execute(
                "INSERT INTO memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.to_string(),
                    "self",
                    entry.content,
                    tags,
                    now,
                    entry.access_count as i64,
                    entry.importance,
                    entry.decay_rate,
                    assoc,
                ],
            )?;

            let parent_id = entry.associations.first().map(|u| u.to_string());

            conn.execute(
                "INSERT INTO self_entries (memory_id, change_type, description, before_state, after_state, reason, approved, parent_id)
                 VALUES (?1, '', '', '', '', '', 0, ?2)",
                params![id.to_string(), parent_id],
            )?;

            Ok(MemoryHandle {
                id,
                memory_type: MemoryType::SelfMemory,
            })
        })
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT m.* FROM memory m WHERE m.memory_type = 'self'",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref text) = query.text {
                sql += &format!(
                    " AND (CAST(m.content AS TEXT) LIKE ?{idx} OR EXISTS (
                        SELECT 1 FROM self_entries se WHERE se.memory_id = m.id
                        AND (se.description LIKE ?{idx} OR se.change_type LIKE ?{idx} OR se.reason LIKE ?{idx})
                    ))",
                    idx = param_idx
                );
                param_values.push(Box::new(format!("%{}%", text)));
                param_idx += 1;
            }

            if let Some(ref tags) = query.tags {
                for tag in tags {
                    sql += &format!(" AND m.tags LIKE ?{idx}", idx = param_idx);
                    param_values.push(Box::new(format!("%{}%", tag)));
                    param_idx += 1;
                }
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

            let mut entries = stmt
                .query_map(params_refs.as_slice(), |row| row_to_entry(row, &self.clock))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Re-sort by activation score (importance + recency + frequency)
            let now = self.clock.wall_now().0 / 1000;
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
            let mut sql = String::from("SELECT * FROM memory WHERE memory_type = 'self'");
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
                .query_map(params_refs.as_slice(), |row| row_to_entry(row, &self.clock))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(entries)
        })
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        self.with_conn(|conn| {
            let id = handle.id.to_string();

            // Only allow forgetting if approved
            let approved: i64 = conn
                .query_row(
                    "SELECT approved FROM self_entries WHERE memory_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .context("self_entry not found")?;

            if approved == 0 {
                bail!(
                    "Cannot forget unapproved self_memory entry {}. \
                     Approval required (approved != 0).",
                    id
                );
            }

            conn.execute("DELETE FROM self_entries WHERE memory_id = ?1", params![id])?;
            conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    async fn compact(&self, _strategy: CompactStrategy) -> Result<CompactResult> {
        // Identity history is permanent — compact is a no-op
        self.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'self'",
                [],
                |r| r.get(0),
            )?;

            Ok(CompactResult {
                entries_before: count as usize,
                entries_after: count as usize,
                entries_removed: 0,
                entries_merged: 0,
            })
        })
    }

    async fn stats(&self) -> Result<MemoryStats> {
        self.with_conn(|conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'self'",
                [],
                |r| r.get(0),
            )?;
            let total_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM memory WHERE memory_type = 'self'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let oldest: Option<String> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM memory WHERE memory_type = 'self'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let newest: Option<String> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM memory WHERE memory_type = 'self'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);

            let mut by_type = std::collections::HashMap::new();
            by_type.insert(MemoryType::SelfMemory, total as usize);

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

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(aletheon_kernel::chronos::TestClock::default())
    }

    fn setup() -> (tempfile::NamedTempFile, SelfMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = SelfMemory::new(tmp.path().to_path_buf(), test_clock());
        (tmp, mem)
    }

    async fn init_mem(mem: &mut SelfMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: None,
        };
        mem.init(&ctx).await.unwrap();
    }

    fn make_identity_change(content: &[u8]) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::SelfMemory,
            content: content.to_vec(),
            tags: vec!["identity".into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 1.0,
            decay_rate: 0.0,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_self_memory_store_and_recall() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_identity_change(b"renamed agent to Aletheon"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("renamed".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_self_memory_lineage_chain() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let root = make_identity_change(b"initial identity");
        let root_id = root.id;
        mem.store(root).await.unwrap();

        let mut child = make_identity_change(b"first evolution");
        child.associations = vec![root_id];
        mem.store(child).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
    }

    #[tokio::test]
    async fn test_self_memory_forget_requires_approval() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_identity_change(b"temporary note");
        let handle = mem.store(entry).await.unwrap();

        // Should fail — not approved
        let result = mem.forget(&handle).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Approval required"));

        // Approve it
        {
            let guard = mem.conn.lock().unwrap();
            let conn = guard.as_ref().unwrap();
            conn.execute(
                "UPDATE self_entries SET approved = 1 WHERE memory_id = ?1",
                params![handle.id.to_string()],
            )
            .unwrap();
        }

        // Now forget should succeed
        mem.forget(&handle).await.unwrap();
    }

    #[tokio::test]
    async fn test_self_memory_compact_is_noop() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_identity_change(b"a")).await.unwrap();
        mem.store(make_identity_change(b"b")).await.unwrap();

        let result = mem
            .compact(CompactStrategy::PruneBelowImportance { threshold: 0.5 })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 2);
        assert_eq!(result.entries_after, 2);
        assert_eq!(result.entries_removed, 0);
    }

    #[tokio::test]
    async fn test_self_memory_stats() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_identity_change(b"first")).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 1);
        assert!(stats.oldest_entry.is_some());
    }
}
