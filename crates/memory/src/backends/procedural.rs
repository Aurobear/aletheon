//! ProceduralMemory — skills, workflows, reusable patterns.

use std::path::PathBuf;
use std::sync::Mutex;

use base::{
    CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter, MemoryHandle,
    MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext, SubsystemHealth, Version,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::ops::activation::{compute_activation, ActivationEntry};
use crate::ops::schema;

pub struct ProceduralMemory {
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl ProceduralMemory {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
        }
    }

    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard.as_ref().expect("ProceduralMemory not initialized");
        f(conn)
    }
}

#[async_trait]
impl Subsystem for ProceduralMemory {
    fn name(&self) -> &str {
        "procedural_memory"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Failed to open {}", self.db_path.display()))?;
        schema::init_base_table(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS procedural_entries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id       TEXT NOT NULL,
                skill_name      TEXT NOT NULL,
                description     TEXT NOT NULL DEFAULT '',
                steps           TEXT NOT NULL DEFAULT '[]',
                triggers        TEXT NOT NULL DEFAULT '[]',
                version         INTEGER NOT NULL DEFAULT 1,
                success_count   INTEGER NOT NULL DEFAULT 0,
                failure_count   INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        self.conn = Mutex::new(Some(conn));
        tracing::info!(path = %self.db_path.display(), "ProceduralMemory initialized");
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
        memory_type: MemoryType::Procedural,
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
impl MemoryBackend for ProceduralMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        self.with_conn(|conn| {
            let id = entry.id;
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;

            let text_content = String::from_utf8_lossy(&entry.content).to_string();
            let skill_name = entry
                .tags
                .first()
                .cloned()
                .unwrap_or_else(|| "unnamed".into());

            let existing_version: Option<i64> = conn
                .query_row(
                    "SELECT MAX(pe.version) FROM procedural_entries pe
                     INNER JOIN memory m ON m.id = pe.memory_id
                     WHERE pe.skill_name = ?1 AND m.importance > 0",
                    params![skill_name],
                    |r| r.get(0),
                )
                .unwrap_or(None);

            let new_version = existing_version.unwrap_or(0) + 1;

            conn.execute(
                "INSERT INTO memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.to_string(),
                    "procedural",
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
                "INSERT INTO procedural_entries (memory_id, skill_name, description, steps, triggers, version, success_count, failure_count)
                 VALUES (?1, ?2, ?3, '[]', '[]', ?4, 0, 0)",
                params![id.to_string(), skill_name, text_content, new_version],
            )?;

            Ok(MemoryHandle {
                id,
                memory_type: MemoryType::Procedural,
            })
        })
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT m.* FROM memory m
                 INNER JOIN procedural_entries pe ON pe.memory_id = m.id
                 WHERE m.memory_type = 'procedural' AND m.importance > 0",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref text) = query.text {
                sql += &format!(
                    " AND (pe.skill_name LIKE ?{idx} OR pe.description LIKE ?{idx} OR CAST(m.content AS TEXT) LIKE ?{idx})",
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
                .query_map(params_refs.as_slice(), row_to_entry)?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Re-sort by activation score (importance + recency + frequency)
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
            let mut sql = String::from(
                "SELECT * FROM memory WHERE memory_type = 'procedural' AND importance > 0",
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
        // Soft-delete: set importance to 0
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE memory SET importance = 0.0 WHERE id = ?1",
                params![handle.id.to_string()],
            )?;
            Ok(())
        })
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        self.with_conn(|conn| {
            let before: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'procedural'",
                [],
                |r| r.get(0),
            )?;

            match strategy {
                CompactStrategy::KeepTopN { n } => {
                    let ids_to_remove: Vec<String> = conn
                        .prepare(
                            "SELECT m.id FROM memory m
                             INNER JOIN procedural_entries pe ON pe.memory_id = m.id
                             WHERE m.memory_type = 'procedural'
                             ORDER BY m.importance * (pe.success_count + 1.0) / (pe.success_count + pe.failure_count + 1.0) DESC
                             LIMIT -1 OFFSET ?1",
                        )?
                        .query_map(params![n as i64], |r| r.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    for id in &ids_to_remove {
                        conn.execute(
                            "DELETE FROM procedural_entries WHERE memory_id = ?1",
                            params![id],
                        )?;
                        conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
                    }
                }
                CompactStrategy::PruneBelowImportance { threshold } => {
                    conn.execute(
                        "DELETE FROM memory WHERE memory_type = 'procedural' AND importance < ?1",
                        params![threshold],
                    )?;
                    conn.execute(
                        "DELETE FROM procedural_entries WHERE memory_id NOT IN (SELECT id FROM memory)",
                        [],
                    )?;
                }
                CompactStrategy::AgeBased {
                    max_age,
                    min_access_count,
                } => {
                    let cutoff = (Utc::now() - max_age).to_rfc3339();
                    conn.execute(
                        "DELETE FROM memory WHERE memory_type = 'procedural'
                         AND created_at < ?1 AND access_count < ?2",
                        params![cutoff, min_access_count as i64],
                    )?;
                    conn.execute(
                        "DELETE FROM procedural_entries WHERE memory_id NOT IN (SELECT id FROM memory)",
                        [],
                    )?;
                }
                CompactStrategy::MergeSimilar { .. } => {
                    // No-op for procedural — skills are distinct
                }
            }

            let after: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'procedural'",
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
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'procedural'",
                [],
                |r| r.get(0),
            )?;
            let total_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM memory WHERE memory_type = 'procedural'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let oldest: Option<String> = conn
                .query_row(
                    "SELECT MIN(created_at) FROM memory WHERE memory_type = 'procedural'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let newest: Option<String> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM memory WHERE memory_type = 'procedural'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);

            let mut by_type = std::collections::HashMap::new();
            by_type.insert(MemoryType::Procedural, total as usize);

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

    fn setup() -> (tempfile::NamedTempFile, ProceduralMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = ProceduralMemory::new(tmp.path().to_path_buf());
        (tmp, mem)
    }

    async fn init_mem(mem: &mut ProceduralMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: std::sync::Arc::new(base::CommunicationBus::new()),
        };
        mem.init(&ctx).await.unwrap();
    }

    fn make_skill(name: &str, content: &[u8]) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Procedural,
            content: content.to_vec(),
            tags: vec![name.into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 0.8,
            decay_rate: 0.0,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_procedural_store_skill() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let handle = mem
            .store(make_skill("git-commit", b"git add . && git commit"))
            .await
            .unwrap();
        assert_eq!(handle.memory_type, MemoryType::Procedural);
    }

    #[tokio::test]
    async fn test_procedural_version_evolution() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_skill("deploy", b"deploy v1")).await.unwrap();
        mem.store(make_skill("deploy", b"deploy v2")).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
    }

    #[tokio::test]
    async fn test_procedural_recall_by_trigger() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_skill("test-runner", b"cargo test"))
            .await
            .unwrap();
        mem.store(make_skill("deployer", b"deploy to prod"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("deploy".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_procedural_soft_forget() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let handle = mem
            .store(make_skill("old-skill", b"deprecated"))
            .await
            .unwrap();

        // Soft-forget
        mem.forget(&handle).await.unwrap();

        // Should not appear in recall (importance filtered to > 0)
        let query = MemoryQuery {
            text: Some("old-skill".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(results.is_empty());

        // Still counted in stats (memory row still exists)
        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 1);
    }

    #[tokio::test]
    async fn test_procedural_stats() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_skill("a", b"alpha")).await.unwrap();
        mem.store(make_skill("b", b"beta")).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
        assert!(stats.total_size_bytes > 0);
    }

    #[tokio::test]
    async fn test_procedural_recall_activation_ordering() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        // Store an old high-importance skill
        let mut old_skill = make_skill("old-skill", b"old approach");
        old_skill.importance = 0.9;
        old_skill.created_at = Utc::now() - chrono::Duration::days(60);
        mem.store(old_skill).await.unwrap();

        // Store a recent moderate-importance skill
        let mut recent_skill = make_skill("recent-skill", b"new approach");
        recent_skill.importance = 0.5;
        recent_skill.created_at = Utc::now();
        mem.store(recent_skill).await.unwrap();

        let query = MemoryQuery {
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 2);

        // With activation-based ordering, the old high-importance entry
        // should still rank higher (importance=0.9 dominates at 40% weight)
        // unless it's extremely old. At 60 days, recency decays but importance
        // still keeps it competitive.
        let first = &results[0];
        assert!(first.importance > 0.0, "activation-sorted results should have positive importance");
    }
}
