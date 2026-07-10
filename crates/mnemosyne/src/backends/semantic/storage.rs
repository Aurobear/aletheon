//! SemanticMemory storage and query operations — full MemoryBackend impl.

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabric::{
    CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter, MemoryHandle,
    MemoryQuery, MemoryStats, MemoryType,
};
use rusqlite::params;
use uuid::Uuid;

use super::query::row_to_entry;
use super::schema::SemanticMemory;
use crate::ops::activation::{compute_activation, ActivationEntry};

#[async_trait::async_trait]
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
                let fetch_limit = if query.limit > 0 { query.limit * 2 } else { 0 };
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

                // Activation-based tiebreaker
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
                let mut indexed: Vec<(usize, &MemoryEntry)> = entries.iter().enumerate().collect();
                indexed.sort_by(|&(i, _), &(j, _)| {
                    let (si, sj) = (scores[i], scores[j]);
                    let max_s = si.max(sj);
                    if max_s > 0.0 && (si - sj).abs() / max_s < 0.1 {
                        std::cmp::Ordering::Equal
                    } else {
                        sj.partial_cmp(&si).unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
                entries = indexed.into_iter().map(|(_, e)| e.clone()).collect();
            } else {
                // Non-FTS path: activation-based sorting
                let mut sql = String::from("SELECT * FROM memory WHERE memory_type = 'semantic'");
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
            let mut sql = String::from("SELECT * FROM memory WHERE memory_type = 'semantic'");
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
