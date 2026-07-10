//! EpisodicMemory storage (write) operations and MemoryBackend trait impl.

use anyhow::Result;
use chrono::Utc;
use fabric::{
    CompactResult, CompactStrategy, EvolutionLogEntry, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType, ReflectionEntry, SelfAwareness,
};
use rusqlite::params;
use uuid::Uuid;

use super::schema::EpisodicMemory;

impl EpisodicMemory {
    /// Store a reflection entry in episodic memory.
    pub fn store_reflection(&self, entry: &ReflectionEntry) -> Result<()> {
        self.with_conn(|conn| {
            let memory_id = Uuid::new_v4().to_string();
            let now = entry.timestamp.to_rfc3339();

            // Also store in base memory table for cross-type recall
            conn.execute(
                "INSERT INTO memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
                 VALUES (?1, 'episodic', ?2, ?3, ?4, 0, 0.8, 0.05, '[]')",
                params![
                    memory_id,
                    entry.to_json_bytes(),
                    format!("[\"reflection\",\"{}\"]", entry.trigger),
                    now,
                ],
            )?;

            conn.execute(
                "INSERT INTO reflection_events (id, memory_id, trigger_type, task_summary, outcome, what_worked, what_failed, learned, behavior_changes, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.id,
                    memory_id,
                    entry.trigger.to_string(),
                    entry.task_summary,
                    entry.outcome.to_string(),
                    serde_json::to_string(&entry.what_worked)?,
                    serde_json::to_string(&entry.what_failed)?,
                    serde_json::to_string(&entry.learned)?,
                    serde_json::to_string(&entry.behavior_changes)?,
                    entry.confidence,
                    now,
                ],
            )?;

            Ok(())
        })
    }

    /// Lower the importance of a base memory entry by memory_id.
    ///
    /// Used by consolidation to soft-archive promoted episodic entries.
    pub fn lower_importance(&self, memory_id: &str, new_importance: f64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE memory SET importance = ?1 WHERE id = ?2",
                params![new_importance, memory_id],
            )?;
            Ok(())
        })
    }

    /// Store a SelfAwareness entry linked to an episodic memory.
    ///
    /// The awareness is stored as a first-class record, not just
    /// serialized bytes in the event. This enables pattern analysis
    /// for growth.
    pub fn store_awareness(&self, memory_id: &str, awareness: &SelfAwareness) -> Result<()> {
        self.with_conn(|conn| {
            let id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();

            conn.execute(
                "INSERT INTO awareness_events (id, memory_id, action, aware, extensions, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    memory_id,
                    awareness.core.action,
                    awareness.core.aware as i32,
                    serde_json::to_string(&awareness.extensions).unwrap_or_default(),
                    now,
                ],
            )?;

            Ok(())
        })
    }

    /// Store an evolution log entry.
    pub fn store_evolution_log(&self, entry: &EvolutionLogEntry) -> Result<()> {
        self.with_conn(|conn| {
            let now = entry.timestamp.to_rfc3339();

            conn.execute(
                "INSERT INTO evolution_log_events (id, trigger, basis, patterns, adjustments, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    entry.id,
                    entry.trigger,
                    serde_json::to_string(&entry.basis)?,
                    serde_json::to_string(&entry.patterns_detected)?,
                    serde_json::to_string(&entry.adjustments)?,
                    now,
                ],
            )?;

            Ok(())
        })
    }
}

#[async_trait::async_trait]
impl MemoryBackend for EpisodicMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        self.with_conn(|conn| {
            let id = entry.id;
            let now = entry.created_at.to_rfc3339();
            let tags = serde_json::to_string(&entry.tags)?;
            let assoc = serde_json::to_string(&entry.associations)?;

            conn.execute(
                "INSERT INTO memory (id, memory_type, content, tags, created_at, access_count, importance, decay_rate, associations)
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
        super::query::recall_impl(self, query)
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        super::query::list_impl(self, filter)
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        self.with_conn(|conn| {
            let id = handle.id.to_string();
            conn.execute(
                "DELETE FROM episodic_events WHERE memory_id = ?1",
                params![id],
            )?;
            conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        self.with_conn(|conn| {
            let before: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'episodic'",
                [],
                |r| r.get(0),
            )?;

            match strategy {
                CompactStrategy::PruneBelowImportance { threshold } => {
                    conn.execute(
                        "DELETE FROM memory WHERE memory_type = 'episodic' AND importance < ?1",
                        params![threshold],
                    )?;
                }
                CompactStrategy::KeepTopN { n } => {
                    conn.execute(
                        "DELETE FROM memory WHERE memory_type = 'episodic' AND id NOT IN (
                            SELECT id FROM memory WHERE memory_type = 'episodic'
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
                        "DELETE FROM memory WHERE memory_type = 'episodic'
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
                "DELETE FROM episodic_events WHERE memory_id NOT IN (SELECT id FROM memory)",
                [],
            )?;

            let after: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'episodic'",
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
        super::query::stats_impl(self)
    }
}
