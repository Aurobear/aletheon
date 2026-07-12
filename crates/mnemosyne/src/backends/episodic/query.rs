//! EpisodicMemory query (read-only) operations.
//!
//! Contains standalone query methods on `EpisodicMemory` and the `row_to_entry`
//! helper used by the `MemoryBackend` impl in `storage.rs`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabric::{
    wall_to_datetime, AwarenessCore, AwarenessExtension, AwarenessExtensionCounts,
    EvolutionLogEntry, MemoryEntry, MemoryFilter, MemoryQuery, MemoryStats, MemoryType,
    ReflectionEntry, ReflectionTrigger, SelfAwareness,
};
use rusqlite::params;
use uuid::Uuid;

use super::schema::EpisodicMemory;
use crate::ops::activation::{compute_activation, ActivationEntry};

impl EpisodicMemory {
    /// Recall recent reflection entries.
    pub fn recall_reflections(&self, limit: usize) -> Result<Vec<ReflectionEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT r.* FROM reflection_events r ORDER BY r.created_at DESC LIMIT ?1",
            )?;

            let entries = stmt
                .query_map(params![limit as i64], |row| {
                    let id: String = row.get("id")?;
                    let created_at: String = row.get("created_at")?;
                    let trigger_str: String = row.get("trigger_type")?;
                    let outcome_str: String = row.get("outcome")?;
                    let what_worked_str: String = row.get("what_worked")?;
                    let what_failed_str: String = row.get("what_failed")?;
                    let learned_str: String = row.get("learned")?;
                    let behavior_changes_str: String = row.get("behavior_changes")?;
                    let confidence: f64 = row.get("confidence")?;
                    let task_summary: String = row.get("task_summary")?;

                    let trigger = match trigger_str.as_str() {
                        "impasse" => ReflectionTrigger::Impasse,
                        "manual" => ReflectionTrigger::Manual,
                        _ => ReflectionTrigger::TaskComplete,
                    };

                    let outcome = match outcome_str.as_str() {
                        "partial" => fabric::ReflectionOutcome::Partial,
                        "failure" => fabric::ReflectionOutcome::Failure,
                        _ => fabric::ReflectionOutcome::Success,
                    };

                    Ok(ReflectionEntry {
                        id,
                        timestamp: created_at.parse().unwrap_or_else(|_| wall_to_datetime(self.clock.wall_now())),
                        trigger,
                        task_summary,
                        outcome,
                        what_worked: serde_json::from_str(&what_worked_str).unwrap_or_default(),
                        what_failed: serde_json::from_str(&what_failed_str).unwrap_or_default(),
                        learned: serde_json::from_str(&learned_str).unwrap_or_default(),
                        behavior_changes: serde_json::from_str(&behavior_changes_str)
                            .unwrap_or_default(),
                        confidence,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            Ok(entries)
        })
    }

    /// Recall reflections with their base-table access count and importance.
    ///
    /// Returns (ReflectionEntry, access_count, importance) tuples, ordered
    /// by most recent first. Used by consolidation to evaluate promotion.
    pub fn recall_reflections_with_access(
        &self,
        limit: usize,
    ) -> Result<Vec<(ReflectionEntry, u64, f64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT r.*, m.access_count, m.importance
                 FROM reflection_events r
                 INNER JOIN memory m ON m.id = r.memory_id
                 ORDER BY r.created_at DESC LIMIT ?1",
            )?;

            let entries = stmt
                .query_map(params![limit as i64], |row| {
                    let id: String = row.get("id")?;
                    let created_at: String = row.get("created_at")?;
                    let trigger_str: String = row.get("trigger_type")?;
                    let outcome_str: String = row.get("outcome")?;
                    let what_worked_str: String = row.get("what_worked")?;
                    let what_failed_str: String = row.get("what_failed")?;
                    let learned_str: String = row.get("learned")?;
                    let behavior_changes_str: String = row.get("behavior_changes")?;
                    let confidence: f64 = row.get("confidence")?;
                    let task_summary: String = row.get("task_summary")?;
                    let access_count: i64 = row.get("access_count")?;
                    let importance: f64 = row.get("importance")?;

                    let trigger = match trigger_str.as_str() {
                        "impasse" => ReflectionTrigger::Impasse,
                        "manual" => ReflectionTrigger::Manual,
                        _ => ReflectionTrigger::TaskComplete,
                    };

                    let outcome = match outcome_str.as_str() {
                        "partial" => fabric::ReflectionOutcome::Partial,
                        "failure" => fabric::ReflectionOutcome::Failure,
                        _ => fabric::ReflectionOutcome::Success,
                    };

                    Ok((
                        ReflectionEntry {
                            id,
                            timestamp: created_at.parse().unwrap_or_else(|_| wall_to_datetime(self.clock.wall_now())),
                            trigger,
                            task_summary,
                            outcome,
                            what_worked: serde_json::from_str(&what_worked_str).unwrap_or_default(),
                            what_failed: serde_json::from_str(&what_failed_str).unwrap_or_default(),
                            learned: serde_json::from_str(&learned_str).unwrap_or_default(),
                            behavior_changes: serde_json::from_str(&behavior_changes_str)
                                .unwrap_or_default(),
                            confidence,
                        },
                        access_count as u64,
                        importance,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            Ok(entries)
        })
    }

    /// Get the current importance of a base memory entry by id.
    pub fn get_importance(&self, memory_id: &str) -> Result<f64> {
        self.with_conn(|conn| {
            let importance: f64 = conn.query_row(
                "SELECT importance FROM memory WHERE id = ?1",
                params![memory_id],
                |r| r.get(0),
            )?;
            Ok(importance)
        })
    }

    /// Look up the memory_id in memory for given reflection event ids.
    ///
    /// Returns a Vec of memory_ids in the same order as the input ids.
    pub fn get_reflection_memory_ids(&self, reflection_ids: Vec<&str>) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let mut memory_ids = Vec::new();
            for rid in reflection_ids {
                let memory_id: String = conn
                    .query_row(
                        "SELECT memory_id FROM reflection_events WHERE id = ?1",
                        params![rid],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                memory_ids.push(memory_id);
            }
            Ok(memory_ids)
        })
    }

    /// Count total reflections stored.
    pub fn reflection_count(&self) -> Result<usize> {
        self.with_conn(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM reflection_events", [], |r| r.get(0))?;
            Ok(count as usize)
        })
    }

    /// Count total evolution log entries stored.
    pub fn evolution_log_count(&self) -> Result<usize> {
        self.with_conn(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM evolution_log_events", [], |r| {
                    r.get(0)
                })?;
            Ok(count as usize)
        })
    }

    /// Recall recent awareness entries for pattern analysis.
    ///
    /// Returns the N most recent awareness entries, ordered by time.
    /// Used by AwarenessGrowthAnalyzer to identify patterns.
    pub fn recall_awareness_history(&self, limit: usize) -> Result<Vec<SelfAwareness>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT action, aware, extensions FROM awareness_events
                 ORDER BY created_at DESC LIMIT ?1",
            )?;

            let entries = stmt
                .query_map(params![limit as i64], |row| {
                    let action: String = row.get("action")?;
                    let aware: bool = row.get::<_, i32>("aware")? != 0;
                    let extensions_str: String = row.get("extensions")?;

                    let extensions: Vec<AwarenessExtension> =
                        serde_json::from_str(&extensions_str).unwrap_or_default();

                    Ok(SelfAwareness {
                        core: AwarenessCore { action, aware },
                        extensions,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            Ok(entries)
        })
    }

    /// Count awareness entries by extension type.
    ///
    /// Returns aggregate counts across all stored awareness entries.
    /// Used to identify which extension types are most/least used.
    pub fn awareness_extension_stats(&self) -> Result<AwarenessExtensionCounts> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT extensions FROM awareness_events")?;

            let mut counts = AwarenessExtensionCounts::default();

            let rows = stmt.query_map([], |row| {
                let extensions_str: String = row.get("extensions")?;
                Ok(extensions_str)
            })?;

            for row in rows {
                let extensions_str = row?;
                let extensions: Vec<AwarenessExtension> =
                    serde_json::from_str(&extensions_str).unwrap_or_default();

                for ext in &extensions {
                    match ext {
                        AwarenessExtension::Intent { .. } => counts.intent += 1,
                        AwarenessExtension::SelfState { .. } => counts.self_state += 1,
                        AwarenessExtension::Significance { .. } => counts.significance += 1,
                        AwarenessExtension::Reflexive { .. } => counts.reflexive += 1,
                    }
                }
            }

            Ok(counts)
        })
    }

    /// Recall recent evolution log entries.
    pub fn recall_evolution_logs(&self, limit: usize) -> Result<Vec<EvolutionLogEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM evolution_log_events ORDER BY created_at DESC LIMIT ?1")?;

            let entries = stmt
                .query_map(params![limit as i64], |row| {
                    let id: String = row.get("id")?;
                    let trigger: String = row.get("trigger")?;
                    let basis_str: String = row.get("basis")?;
                    let patterns_str: String = row.get("patterns")?;
                    let adjustments_str: String = row.get("adjustments")?;
                    let created_at: String = row.get("created_at")?;

                    Ok(EvolutionLogEntry {
                        id,
                        timestamp: created_at.parse().unwrap_or_else(|_| wall_to_datetime(self.clock.wall_now())),
                        trigger,
                        basis: serde_json::from_str(&basis_str).unwrap_or_default(),
                        patterns_detected: serde_json::from_str(&patterns_str).unwrap_or_default(),
                        adjustments: serde_json::from_str(&adjustments_str).unwrap_or_default(),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            Ok(entries)
        })
    }
}

/// Convert a rusqlite Row into a MemoryEntry.
pub(super) fn row_to_entry(row: &rusqlite::Row, clock: &std::sync::Arc<dyn fabric::Clock>) -> rusqlite::Result<MemoryEntry> {
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
            .unwrap_or_else(|_| wall_to_datetime(clock.wall_now())),
        access_count: row.get::<_, i64>("access_count")? as u64,
        importance: row.get("importance")?,
        decay_rate: row.get("decay_rate")?,
        associations: serde_json::from_str(&assoc_str).unwrap_or_default(),
    })
}

/// Implementation of `recall` for the `MemoryBackend` trait.
/// Called from `storage.rs` where the trait is implemented.
pub(super) fn recall_impl(mem: &EpisodicMemory, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
    mem.with_conn(|conn| {
        let mut sql = String::from(
            "SELECT m.* FROM memory m WHERE m.memory_type = 'episodic'",
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

        let effective_limit = if query.limit > 0 {
            let fetch_limit = (query.limit as i64) * 2;
            sql += &format!(" LIMIT ?{idx}", idx = param_idx);
            param_values.push(Box::new(fetch_limit));
            Some(query.limit)
        } else {
            None
        };

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut entries = stmt
            .query_map(params_refs.as_slice(), |row| row_to_entry(row, &mem.clock))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let now = mem.clock.wall_now().0 / 1000;
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

        if let Some(limit) = effective_limit {
            entries.truncate(limit);
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

/// Implementation of `list` for the `MemoryBackend` trait.
pub(super) fn list_impl(mem: &EpisodicMemory, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
    mem.with_conn(|conn| {
        let mut sql = String::from("SELECT * FROM memory WHERE memory_type = 'episodic'");
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
            .query_map(params_refs.as_slice(), |row| row_to_entry(row, &mem.clock))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    })
}

/// Implementation of `stats` for the `MemoryBackend` trait.
pub(super) fn stats_impl(mem: &EpisodicMemory) -> Result<MemoryStats> {
    mem.with_conn(|conn| {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'episodic'",
            [],
            |r| r.get(0),
        )?;
        let total_size: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM memory WHERE memory_type = 'episodic'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let oldest: Option<String> = conn
            .query_row(
                "SELECT MIN(created_at) FROM memory WHERE memory_type = 'episodic'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(None);
        let newest: Option<String> = conn
            .query_row(
                "SELECT MAX(created_at) FROM memory WHERE memory_type = 'episodic'",
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
