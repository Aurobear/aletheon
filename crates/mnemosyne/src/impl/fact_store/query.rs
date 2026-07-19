use anyhow::{Context, Result};

use super::{
    sanitize_fts_query, ConsolidationLogRow, EpisodeRow, FactRow, FactStore, FeedbackResult,
    KnowledgeRow, DEFAULT_MIN_TRUST, HELPFUL_DELTA, STALE_DAYS, STALE_DECAY_DELTA, TRUST_MAX,
    TRUST_MIN, UNHELPFUL_DELTA,
};

impl FactStore {
    // ── Core Fact CRUD ───────────────────────────────────────────────────────

    /// Add a fact. INSERT OR IGNORE on duplicate content.
    /// Returns the fact_id (existing or newly inserted).
    #[allow(clippy::too_many_arguments)]
    pub fn add_fact(
        &self,
        content: &str,
        category: &str,
        tags: &str,
        source_path: &str,
        trust: f64,
        tier: &str,
        ttl_days: i64,
    ) -> Result<i64> {
        self.db.execute(
            "INSERT OR IGNORE INTO facts (content, category, tags, source_path, trust_score, tier, ttl_days)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![content, category, tags, source_path, trust, tier, ttl_days],
        )?;
        let fact_id: i64 = self.db.query_row(
            "SELECT fact_id FROM facts WHERE content = ?1",
            rusqlite::params![content],
            |row| row.get(0),
        )?;

        // Extract and link entities
        let entities = Self::extract_entities(content);
        for entity_name in entities {
            let entity_id = self.resolve_entity(&entity_name)?;
            self.link_fact_entity(fact_id, entity_id)?;
        }

        Ok(fact_id)
    }

    /// Full-text search over facts.
    /// Increments retrieval_count for returned facts.
    pub fn search_facts(
        &self,
        query: &str,
        category: Option<&str>,
        min_trust: f64,
        limit: usize,
    ) -> Result<Vec<FactRow>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = sanitize_fts_query(query);
        let min_trust = if min_trust <= 0.0 {
            DEFAULT_MIN_TRUST
        } else {
            min_trust
        };

        let sql = match category {
            Some(_) => {
                "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                        f.trust_score, f.retrieval_count, f.helpful_count,
                        f.tier, f.ttl_days, f.created_at, f.updated_at,
                        f.scope, f.source, f.status, f.pinned, f.subject
                 FROM facts f
                 INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
                 WHERE facts_fts MATCH ?1
                   AND f.trust_score >= ?2
                   AND f.category = ?3
                 ORDER BY rank
                 LIMIT ?4"
            }
            None => {
                "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                        f.trust_score, f.retrieval_count, f.helpful_count,
                        f.tier, f.ttl_days, f.created_at, f.updated_at,
                        f.scope, f.source, f.status, f.pinned, f.subject
                 FROM facts f
                 INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
                 WHERE facts_fts MATCH ?1
                   AND f.trust_score >= ?2
                 ORDER BY rank
                 LIMIT ?3"
            }
        };

        let mut stmt = self.db.prepare(sql)?;
        let rows = if let Some(cat) = category {
            stmt.query_map(
                rusqlite::params![fts_query, min_trust, cat, limit as i64],
                Self::map_fact_row,
            )?
        } else {
            stmt.query_map(
                rusqlite::params![fts_query, min_trust, limit as i64],
                Self::map_fact_row,
            )?
        };

        let results: Vec<FactRow> = rows.collect::<Result<Vec<_>, _>>()?;

        // Increment retrieval_count for matched facts
        for fact in &results {
            self.db.execute(
                "UPDATE facts SET retrieval_count = retrieval_count + 1 WHERE fact_id = ?1",
                rusqlite::params![fact.fact_id],
            )?;
        }

        Ok(results)
    }

    /// Governed FTS query. The effective scope of a fact is the same one used
    /// by recall materialization: a non-empty subject is Principal scope;
    /// otherwise it belongs to the requesting Session. The SQL predicate runs
    /// before `FactRow` candidates are constructed.
    pub fn search_facts_prefiltered(
        &self,
        query: &str,
        session_id: &str,
        min_trust: f64,
        limit: usize,
        predicate: &crate::ScopePredicate,
    ) -> Result<Vec<FactRow>> {
        if query.trim().is_empty()
            || !predicate.allows_authority(crate::MemoryAuthority::VerifiedLocalSemantic)
            || !predicate.allows_sensitivity(crate::MemorySensitivity::Internal)
        {
            return Ok(Vec::new());
        }
        let session_allowed =
            predicate.allows_scope(&crate::MemoryScope::Session(session_id.to_string()));
        let principal_scope_keys = predicate
            .scope_keys
            .iter()
            .filter(|key| key.starts_with("principal:"))
            .cloned()
            .collect::<Vec<_>>();
        if !session_allowed && principal_scope_keys.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = sanitize_fts_query(query);
        let min_trust = if min_trust <= 0.0 {
            DEFAULT_MIN_TRUST
        } else {
            min_trust
        };
        let principal_scope_keys = serde_json::to_string(&principal_scope_keys)?;
        let mut stmt = self.db.prepare(
            "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                    f.trust_score, f.retrieval_count, f.helpful_count,
                    f.tier, f.ttl_days, f.created_at, f.updated_at,
                    f.scope, f.source, f.status, f.pinned, f.subject
             FROM facts f
             INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
             WHERE facts_fts MATCH ?1
               AND f.trust_score >= ?2
               AND ((f.subject = '' AND ?3 = 1)
                    OR ('principal:' || f.subject) IN
                       (SELECT value FROM json_each(?4)))
             ORDER BY rank
             LIMIT ?5",
        )?;
        let results = stmt
            .query_map(
                rusqlite::params![
                    fts_query,
                    min_trust,
                    i64::from(session_allowed),
                    principal_scope_keys,
                    limit as i64
                ],
                Self::map_fact_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        for fact in &results {
            self.db.execute(
                "UPDATE facts SET retrieval_count = retrieval_count + 1 WHERE fact_id = ?1",
                rusqlite::params![fact.fact_id],
            )?;
        }
        Ok(results)
    }

    /// Record user feedback on a fact.
    pub fn record_feedback(&self, fact_id: i64, helpful: bool) -> Result<FeedbackResult> {
        let old_trust: f64 = self
            .db
            .query_row(
                "SELECT trust_score FROM facts WHERE fact_id = ?1",
                rusqlite::params![fact_id],
                |row| row.get(0),
            )
            .context("fact not found")?;

        let delta = if helpful {
            HELPFUL_DELTA
        } else {
            UNHELPFUL_DELTA
        };
        let new_trust = (old_trust + delta).clamp(TRUST_MIN, TRUST_MAX);

        self.db.execute(
            "UPDATE facts SET trust_score = ?1, helpful_count = helpful_count + ?2, updated_at = datetime('now')
             WHERE fact_id = ?3",
            rusqlite::params![new_trust, if helpful { 1i64 } else { 0i64 }, fact_id],
        )?;

        let helpful_count: i64 = self.db.query_row(
            "SELECT helpful_count FROM facts WHERE fact_id = ?1",
            rusqlite::params![fact_id],
            |row| row.get(0),
        )?;

        Ok(FeedbackResult {
            fact_id,
            old_trust,
            new_trust,
            helpful_count,
        })
    }

    /// Decay trust for facts not retrieved in 7+ days.
    pub fn decay_stale(&self) -> Result<usize> {
        let affected = self.db.execute(
            "UPDATE facts SET trust_score = MAX(?1, trust_score + ?2), updated_at = datetime('now')
             WHERE trust_score > ?1
               AND updated_at < datetime('now', ?3)",
            rusqlite::params![
                TRUST_MIN,
                STALE_DECAY_DELTA,
                format!("-{} days", STALE_DAYS)
            ],
        )?;
        Ok(affected)
    }

    /// Get a single fact by ID.
    pub fn get_fact(&self, fact_id: i64) -> Result<Option<FactRow>> {
        let mut stmt = self.db.prepare(
            "SELECT fact_id, content, category, tags, source_path,
                    trust_score, retrieval_count, helpful_count,
                    tier, ttl_days, created_at, updated_at,
                    scope, source, status, pinned, subject
             FROM facts WHERE fact_id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![fact_id], Self::map_fact_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Delete a fact. Returns true if a row was deleted.
    pub fn delete_fact(&self, fact_id: i64) -> Result<bool> {
        let affected = self.db.execute(
            "DELETE FROM facts WHERE fact_id = ?1",
            rusqlite::params![fact_id],
        )?;
        Ok(affected > 0)
    }

    // ── Governed Write Path ──────────────────────────────────────────────────

    /// Add a fact with governance fields. Delegates to the same INSERT
    /// but includes scope/source/subject. Checks secret-safety unless
    /// source == "explicit" (user deliberately storing it).
    #[allow(clippy::too_many_arguments)]
    pub fn add_fact_governed(
        &self,
        content: &str,
        category: &str,
        tags: &str,
        scope: &str,
        source: &str,
        subject: &str,
        trust: f64,
        tier: &str,
        ttl_days: i64,
    ) -> Result<i64> {
        if source != "explicit" && super::is_sensitive(content) {
            anyhow::bail!("refused to store likely-sensitive content (source={source})");
        }
        self.db.execute(
            "INSERT OR IGNORE INTO facts
               (content, category, tags, source_path, trust_score, tier, ttl_days, scope, source, subject)
             VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![content, category, tags, trust, tier, ttl_days, scope, source, subject],
        )?;
        let fact_id: i64 = self.db.query_row(
            "SELECT fact_id FROM facts WHERE content = ?1",
            rusqlite::params![content],
            |r| r.get(0),
        )?;
        // Extract and link entities (same as legacy add_fact)
        let entities = Self::extract_entities(content);
        for entity_name in entities {
            let eid = self.resolve_entity(&entity_name)?;
            self.link_fact_entity(fact_id, eid)?;
        }
        Ok(fact_id)
    }

    /// Scope/status/ttl-aware search with pinned boost.
    pub fn search_facts_governed(
        &self,
        query: &str,
        scope: Option<&str>,
        include_archived: bool,
        min_trust: f64,
        limit: usize,
    ) -> Result<Vec<FactRow>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts = super::sanitize_fts_query(query);
        let min_trust = if min_trust <= 0.0 {
            super::DEFAULT_MIN_TRUST
        } else {
            min_trust
        };
        let mut sql = String::from(
            "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                    f.trust_score, f.retrieval_count, f.helpful_count,
                    f.tier, f.ttl_days, f.created_at, f.updated_at,
                    f.scope, f.source, f.status, f.pinned, f.subject
             FROM facts f INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
             WHERE facts_fts MATCH ?1 AND f.trust_score >= ?2
               AND (f.ttl_days = 0 OR f.created_at >= datetime('now', '-' || f.ttl_days || ' days'))",
        );
        if !include_archived {
            sql.push_str(" AND f.status = 'active'");
        }
        if scope.is_some() {
            sql.push_str(" AND f.scope = ?3");
        }
        sql.push_str(" ORDER BY f.pinned DESC, rank LIMIT ?LIM");
        let sql = sql.replace("?LIM", if scope.is_some() { "?4" } else { "?3" });

        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = scope {
            stmt.query_map(
                rusqlite::params![fts, min_trust, s, limit as i64],
                Self::map_fact_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(
                rusqlite::params![fts, min_trust, limit as i64],
                Self::map_fact_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        for f in &rows {
            self.db.execute(
                "UPDATE facts SET retrieval_count = retrieval_count + 1 WHERE fact_id = ?1",
                rusqlite::params![f.fact_id],
            )?;
        }
        Ok(rows)
    }

    pub fn set_pinned(&self, fact_id: i64, pinned: bool) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE facts SET pinned = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
            rusqlite::params![pinned as i64, fact_id],
        )? > 0)
    }

    pub fn set_status(&self, fact_id: i64, status: &str) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE facts SET status = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
            rusqlite::params![status, fact_id],
        )? > 0)
    }

    pub fn list_facts(
        &self,
        scope: Option<&str>,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<FactRow>> {
        let mut sql = String::from(
            "SELECT fact_id, content, category, tags, source_path,
                    trust_score, retrieval_count, helpful_count,
                    tier, ttl_days, created_at, updated_at,
                    scope, source, status, pinned, subject
             FROM facts WHERE 1=1",
        );
        if !include_archived {
            sql.push_str(" AND status = 'active'");
        }
        if scope.is_some() {
            sql.push_str(" AND scope = ?1");
        }
        sql.push_str(&format!(
            " ORDER BY pinned DESC, updated_at DESC LIMIT {}",
            limit as i64
        ));
        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = scope {
            stmt.query_map(rusqlite::params![s], Self::map_fact_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], Self::map_fact_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    // ── Episodes ─────────────────────────────────────────────────────────────

    /// Add an episode.
    #[allow(clippy::too_many_arguments)]
    pub fn add_episode(
        &self,
        session_id: &str,
        task: &str,
        context_json: &str,
        actions_json: &str,
        outcome: &str,
        outcome_detail: &str,
        importance: f64,
    ) -> Result<i64> {
        self.db.execute(
            "INSERT INTO episodes (session_id, task, context_json, actions_json, outcome, outcome_detail, importance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                session_id,
                task,
                context_json,
                actions_json,
                outcome,
                outcome_detail,
                importance
            ],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Full-text search over episodes.
    pub fn search_episodes(&self, query: &str, limit: usize) -> Result<Vec<EpisodeRow>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = sanitize_fts_query(query);
        let mut stmt = self.db.prepare(
            "SELECT e.episode_id, e.session_id, e.task, e.context_json, e.actions_json,
                    e.outcome, e.outcome_detail, e.importance, e.consolidated, e.timestamp
             FROM episodes e
             INNER JOIN episodes_fts fts ON e.episode_id = fts.rowid
             WHERE episodes_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![fts_query, limit as i64], |row| {
            Ok(EpisodeRow {
                episode_id: row.get(0)?,
                session_id: row.get(1)?,
                task: row.get(2)?,
                context_json: row.get(3)?,
                actions_json: row.get(4)?,
                outcome: row.get(5)?,
                outcome_detail: row.get(6)?,
                importance: row.get(7)?,
                consolidated: row.get::<_, i64>(8)? != 0,
                timestamp: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get episodes not yet consolidated.
    pub fn get_unconsolidated_episodes(&self, limit: usize) -> Result<Vec<EpisodeRow>> {
        let mut stmt = self.db.prepare(
            "SELECT episode_id, session_id, task, context_json, actions_json,
                    outcome, outcome_detail, importance, consolidated, timestamp
             FROM episodes
             WHERE consolidated = 0
             ORDER BY importance DESC, timestamp DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(EpisodeRow {
                episode_id: row.get(0)?,
                session_id: row.get(1)?,
                task: row.get(2)?,
                context_json: row.get(3)?,
                actions_json: row.get(4)?,
                outcome: row.get(5)?,
                outcome_detail: row.get(6)?,
                importance: row.get(7)?,
                consolidated: row.get::<_, i64>(8)? != 0,
                timestamp: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Mark episodes as consolidated.
    pub fn mark_consolidated(&self, ids: &[i64]) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE episodes SET consolidated = 1 WHERE episode_id = ?1",
                rusqlite::params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Count episodes, optionally filtered by outcome.
    pub fn count_episodes(&self, outcome: Option<&str>) -> Result<i64> {
        let count = match outcome {
            Some(o) => self.db.query_row(
                "SELECT COUNT(*) FROM episodes WHERE outcome = ?1",
                rusqlite::params![o],
                |row| row.get::<_, i64>(0),
            )?,
            None => self
                .db
                .query_row("SELECT COUNT(*) FROM episodes", [], |row| {
                    row.get::<_, i64>(0)
                })?,
        };
        Ok(count)
    }

    // ── Knowledge ────────────────────────────────────────────────────────────

    /// Add a knowledge entry.
    pub fn add_knowledge(
        &self,
        topic: &str,
        content: &str,
        source_episodes: &str,
        confidence: f64,
    ) -> Result<i64> {
        self.db.execute(
            "INSERT INTO knowledge (topic, content, source_episodes, confidence)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![topic, content, source_episodes, confidence],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Full-text search over knowledge. Increments access_count.
    pub fn search_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeRow>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = sanitize_fts_query(query);
        let mut stmt = self.db.prepare(
            "SELECT k.knowledge_id, k.topic, k.content, k.source_episodes,
                    k.confidence, k.access_count, k.created_at
             FROM knowledge k
             INNER JOIN knowledge_fts fts ON k.knowledge_id = fts.rowid
             WHERE knowledge_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![fts_query, limit as i64], |row| {
            Ok(KnowledgeRow {
                knowledge_id: row.get(0)?,
                topic: row.get(1)?,
                content: row.get(2)?,
                source_episodes: row.get(3)?,
                confidence: row.get(4)?,
                access_count: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        let results: Vec<KnowledgeRow> = rows.collect::<Result<Vec<_>, _>>()?;

        for k in &results {
            self.db.execute(
                "UPDATE knowledge SET access_count = access_count + 1 WHERE knowledge_id = ?1",
                rusqlite::params![k.knowledge_id],
            )?;
        }

        Ok(results)
    }

    // ── Consolidation Log ────────────────────────────────────────────────────

    /// Log a consolidation run.
    pub fn log_consolidation(
        &self,
        episodes_processed: i64,
        knowledge_extracted: i64,
        errors: &str,
    ) -> Result<()> {
        self.db.execute(
            "INSERT INTO consolidation_log (episodes_processed, knowledge_extracted, errors)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![episodes_processed, knowledge_extracted, errors],
        )?;
        Ok(())
    }

    /// Get the most recent consolidation log entry.
    pub fn get_last_consolidation(&self) -> Result<Option<ConsolidationLogRow>> {
        let mut stmt = self.db.prepare(
            "SELECT log_id, run_at, episodes_processed, knowledge_extracted, errors
             FROM consolidation_log
             ORDER BY log_id DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(ConsolidationLogRow {
                log_id: row.get(0)?,
                run_at: row.get(1)?,
                episodes_processed: row.get(2)?,
                knowledge_extracted: row.get(3)?,
                errors: row.get(4)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
}
