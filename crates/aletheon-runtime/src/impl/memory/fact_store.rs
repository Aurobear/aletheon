use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;

// ── Trust scoring constants ──────────────────────────────────────────────────
const HELPFUL_DELTA: f64 = 0.05;
const UNHELPFUL_DELTA: f64 = -0.10;
const TRUST_MIN: f64 = 0.0;
const TRUST_MAX: f64 = 1.0;
/// Default trust score for new facts — reserved for future trust-weighted retrieval.
#[allow(dead_code)]
const DEFAULT_TRUST: f64 = 0.5;
const DEFAULT_MIN_TRUST: f64 = 0.3;
const STALE_DECAY_DELTA: f64 = -0.002;
const STALE_DAYS: i64 = 7;

// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactRow {
    pub fact_id: i64,
    pub content: String,
    pub category: String,
    pub tags: String,
    pub source_path: String,
    pub trust_score: f64,
    pub retrieval_count: i64,
    pub helpful_count: i64,
    pub tier: String,
    pub ttl_days: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct FeedbackResult {
    pub fact_id: i64,
    pub old_trust: f64,
    pub new_trust: f64,
    pub helpful_count: i64,
}

#[derive(Debug, Clone)]
pub struct EntityNeighbor {
    pub entity_id: i64,
    pub name: String,
    pub shared_facts: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EpisodeRow {
    pub episode_id: i64,
    pub session_id: String,
    pub task: String,
    pub context_json: String,
    pub actions_json: String,
    pub outcome: String,
    pub outcome_detail: String,
    pub importance: f64,
    pub consolidated: bool,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeRow {
    pub knowledge_id: i64,
    pub topic: String,
    pub content: String,
    pub source_episodes: String,
    pub confidence: f64,
    pub access_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ConsolidationLogRow {
    pub log_id: i64,
    pub run_at: String,
    pub episodes_processed: i64,
    pub knowledge_extracted: i64,
    pub errors: String,
}

// ── FactStore ────────────────────────────────────────────────────────────────

/// SQLite-backed fact store with trust scoring, entity graph, episodes,
/// and knowledge storage. FTS5 for full-text search.
pub struct FactStore {
    db: Connection,
}

impl FactStore {
    /// Open (or create) a FactStore at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening fact store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::create_schema(&db)?;
        Ok(Self { db })
    }

    fn create_schema(db: &Connection) -> Result<()> {
        // ── facts table ──────────────────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS facts (
                fact_id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL UNIQUE,
                category TEXT NOT NULL DEFAULT 'general',
                tags TEXT NOT NULL DEFAULT '',
                source_path TEXT NOT NULL DEFAULT '',
                trust_score REAL NOT NULL DEFAULT 0.5,
                retrieval_count INTEGER NOT NULL DEFAULT 0,
                helpful_count INTEGER NOT NULL DEFAULT 0,
                tier TEXT NOT NULL DEFAULT 'episodic',
                ttl_days INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_facts_trust ON facts(trust_score DESC);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);
            CREATE INDEX IF NOT EXISTS idx_facts_tier ON facts(tier);",
        )?;

        // ── facts FTS5 + sync triggers ───────────────────────────────────────
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(
                content, tags,
                content=facts, content_rowid=fact_id,
                tokenize='porter unicode61'
            );",
        )?;
        db.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
                INSERT INTO facts_fts(rowid, content, tags) VALUES (new.fact_id, new.content, new.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
                INSERT INTO facts_fts(facts_fts, rowid, content, tags) VALUES('delete', old.fact_id, old.content, old.tags);
            END;
            CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE ON facts BEGIN
                INSERT INTO facts_fts(facts_fts, rowid, content, tags) VALUES('delete', old.fact_id, old.content, old.tags);
                INSERT INTO facts_fts(rowid, content, tags) VALUES (new.fact_id, new.content, new.tags);
            END;",
        )?;

        // ── entities table ───────────────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS entities (
                entity_id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                aliases TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name);",
        )?;

        // ── fact_entities join table ─────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS fact_entities (
                fact_id INTEGER NOT NULL REFERENCES facts(fact_id) ON DELETE CASCADE,
                entity_id INTEGER NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
                PRIMARY KEY (fact_id, entity_id)
            );",
        )?;

        // ── episodes table ───────────────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS episodes (
                episode_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                task TEXT NOT NULL,
                context_json TEXT NOT NULL DEFAULT '{}',
                actions_json TEXT NOT NULL DEFAULT '[]',
                outcome TEXT NOT NULL DEFAULT 'success' CHECK(outcome IN ('success','failure','partial','abandoned')),
                outcome_detail TEXT NOT NULL DEFAULT '',
                importance REAL NOT NULL DEFAULT 0.5,
                consolidated INTEGER NOT NULL DEFAULT 0,
                timestamp TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_outcome ON episodes(outcome);
            CREATE INDEX IF NOT EXISTS idx_episodes_consolidated ON episodes(consolidated);
            CREATE INDEX IF NOT EXISTS idx_episodes_timestamp ON episodes(timestamp DESC);",
        )?;

        // ── episodes FTS5 + sync triggers ────────────────────────────────────
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS episodes_fts USING fts5(
                task, context_json,
                content=episodes, content_rowid=episode_id,
                tokenize='porter unicode61'
            );",
        )?;
        db.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS episodes_ai AFTER INSERT ON episodes BEGIN
                INSERT INTO episodes_fts(rowid, task, context_json) VALUES (new.episode_id, new.task, new.context_json);
            END;
            CREATE TRIGGER IF NOT EXISTS episodes_ad AFTER DELETE ON episodes BEGIN
                INSERT INTO episodes_fts(episodes_fts, rowid, task, context_json) VALUES('delete', old.episode_id, old.task, old.context_json);
            END;",
        )?;

        // ── knowledge table ──────────────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge (
                knowledge_id INTEGER PRIMARY KEY AUTOINCREMENT,
                topic TEXT NOT NULL,
                content TEXT NOT NULL,
                source_episodes TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 0.5,
                access_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_topic ON knowledge(topic);",
        )?;

        // ── knowledge FTS5 + sync triggers ───────────────────────────────────
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts USING fts5(
                topic, content,
                content=knowledge, content_rowid=knowledge_id,
                tokenize='porter unicode61'
            );",
        )?;
        db.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS knowledge_ai AFTER INSERT ON knowledge BEGIN
                INSERT INTO knowledge_fts(rowid, topic, content) VALUES (new.knowledge_id, new.topic, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS knowledge_ad AFTER DELETE ON knowledge BEGIN
                INSERT INTO knowledge_fts(knowledge_fts, rowid, topic, content) VALUES('delete', old.knowledge_id, old.topic, old.content);
            END;",
        )?;

        // ── consolidation_log table ──────────────────────────────────────────
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS consolidation_log (
                log_id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_at TEXT NOT NULL DEFAULT (datetime('now')),
                episodes_processed INTEGER NOT NULL DEFAULT 0,
                knowledge_extracted INTEGER NOT NULL DEFAULT 0,
                errors TEXT NOT NULL DEFAULT ''
            );",
        )?;

        Ok(())
    }

    // ── Core Fact CRUD ───────────────────────────────────────────────────────

    /// Add a fact. INSERT OR IGNORE on duplicate content.
    /// Returns the fact_id (existing or newly inserted).
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
                        f.tier, f.ttl_days, f.created_at, f.updated_at
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
                        f.tier, f.ttl_days, f.created_at, f.updated_at
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
            rusqlite::params![TRUST_MIN, STALE_DECAY_DELTA, format!("-{} days", STALE_DAYS)],
        )?;
        Ok(affected)
    }

    /// Get a single fact by ID.
    pub fn get_fact(&self, fact_id: i64) -> Result<Option<FactRow>> {
        let mut stmt = self.db.prepare(
            "SELECT fact_id, content, category, tags, source_path,
                    trust_score, retrieval_count, helpful_count,
                    tier, ttl_days, created_at, updated_at
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

    // ── Entity Graph ─────────────────────────────────────────────────────────

    /// Extract entity names from content using regex patterns:
    /// capitalized multi-word phrases, double-quoted, single-quoted, and "X aka Y".
    pub fn extract_entities(content: &str) -> Vec<String> {
        let mut entities = Vec::new();

        // Capitalized multi-word: two or more consecutive capitalized words
        let cap_re = Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)+)\b").unwrap();
        for cap in cap_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // Capitalized single words not mid-sentence (after . ! ?).
        // Matches any [A-Z][a-z]+ word, then we filter out those that appear
        // right after a sentence boundary (period/exclaim/question + space).
        let single_re = Regex::new(r"\b([A-Z][a-z]+)\b").unwrap();
        for m in single_re.find_iter(content) {
            let before = &content[..m.start()];
            let is_mid_sentence_start = before.ends_with(". ")
                || before.ends_with("! ")
                || before.ends_with("? ");
            if !is_mid_sentence_start {
                entities.push(m.as_str().to_string());
            }
        }

        // Double-quoted terms
        let dq_re = Regex::new(r#""([^"]{2,})""#).unwrap();
        for cap in dq_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // Single-quoted terms
        let sq_re = Regex::new(r"'([^']{2,})'").unwrap();
        for cap in sq_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // "X aka Y" pattern
        let aka_re = Regex::new(r"\b(\w[\w\s]+?)\s+aka\s+(\w[\w\s]+?)(?:\s|,|\.|$)").unwrap();
        for cap in aka_re.captures_iter(content) {
            let a = cap[1].trim().to_string();
            let b = cap[2].trim().to_string();
            if !a.is_empty() {
                entities.push(a);
            }
            if !b.is_empty() {
                entities.push(b);
            }
        }

        entities.sort();
        entities.dedup();
        entities
    }

    /// Resolve an entity name to its ID. Creates if not found.
    pub fn resolve_entity(&self, name: &str) -> Result<i64> {
        // Try exact match
        if let Ok(id) = self.db.query_row(
            "SELECT entity_id FROM entities WHERE name = ?1",
            rusqlite::params![name],
            |row| row.get::<_, i64>(0),
        ) {
            return Ok(id);
        }

        // Try alias match
        if let Ok(id) = self.db.query_row(
            "SELECT entity_id FROM entities WHERE ',' || aliases || ',' LIKE ?1",
            rusqlite::params![format!(",{},", name)],
            |row| row.get::<_, i64>(0),
        ) {
            return Ok(id);
        }

        // Create new entity
        self.db.execute(
            "INSERT INTO entities (name) VALUES (?1)",
            rusqlite::params![name],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Link a fact to an entity.
    pub fn link_fact_entity(&self, fact_id: i64, entity_id: i64) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO fact_entities (fact_id, entity_id) VALUES (?1, ?2)",
            rusqlite::params![fact_id, entity_id],
        )?;
        Ok(())
    }

    /// Get 1-hop neighbors of an entity via shared facts.
    pub fn get_entity_neighbors(&self, entity_id: i64) -> Result<Vec<EntityNeighbor>> {
        let mut stmt = self.db.prepare(
            "SELECT e.entity_id, e.name, COUNT(DISTINCT fe2.fact_id) as shared_facts
             FROM fact_entities fe1
             JOIN fact_entities fe2 ON fe1.fact_id = fe2.fact_id AND fe2.entity_id != ?1
             JOIN entities e ON fe2.entity_id = e.entity_id
             WHERE fe1.entity_id = ?1
             GROUP BY e.entity_id, e.name
             ORDER BY shared_facts DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![entity_id], |row| {
            Ok(EntityNeighbor {
                entity_id: row.get(0)?,
                name: row.get(1)?,
                shared_facts: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// BFS path between two entities, up to max_depth hops.
    pub fn find_entity_path(
        &self,
        from_id: i64,
        to_id: i64,
        max_depth: usize,
    ) -> Result<Option<Vec<i64>>> {
        use std::collections::{HashSet, VecDeque};

        if from_id == to_id {
            return Ok(Some(vec![from_id]));
        }

        let mut visited = HashSet::new();
        visited.insert(from_id);
        let mut queue = VecDeque::new();
        queue.push_back(vec![from_id]);

        while let Some(path) = queue.pop_front() {
            if path.len() > max_depth {
                continue;
            }
            let current = *path.last().unwrap();
            let neighbors = self.get_entity_neighbors(current)?;
            for nb in neighbors {
                if nb.entity_id == to_id {
                    let mut result = path;
                    result.push(to_id);
                    return Ok(Some(result));
                }
                if !visited.contains(&nb.entity_id) {
                    visited.insert(nb.entity_id);
                    let mut new_path = path.clone();
                    new_path.push(nb.entity_id);
                    queue.push_back(new_path);
                }
            }
        }
        Ok(None)
    }

    /// Get all facts linked to an entity.
    pub fn get_entity_facts(&self, entity_id: i64) -> Result<Vec<FactRow>> {
        let mut stmt = self.db.prepare(
            "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                    f.trust_score, f.retrieval_count, f.helpful_count,
                    f.tier, f.ttl_days, f.created_at, f.updated_at
             FROM facts f
             JOIN fact_entities fe ON f.fact_id = fe.fact_id
             WHERE fe.entity_id = ?1
             ORDER BY f.trust_score DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![entity_id], Self::map_fact_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ── Episodes ─────────────────────────────────────────────────────────────

    /// Add an episode.
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

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn map_fact_row(row: &rusqlite::Row) -> rusqlite::Result<FactRow> {
        Ok(FactRow {
            fact_id: row.get(0)?,
            content: row.get(1)?,
            category: row.get(2)?,
            tags: row.get(3)?,
            source_path: row.get(4)?,
            trust_score: row.get(5)?,
            retrieval_count: row.get(6)?,
            helpful_count: row.get(7)?,
            tier: row.get(8)?,
            ttl_days: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

/// Sanitize a user query for FTS5 MATCH.
fn sanitize_fts_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                return String::new();
            }
            format!("\"{}*\"", clean)
        })
        .filter(|s| !s.is_empty())
        .collect();

    if words.is_empty() {
        return "\"\"".to_string();
    }
    words.join(" OR ")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup() -> (FactStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = FactStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    // ── Schema tests ────────────────────────────────────────────────────────

    #[test]
    fn test_schema_creation() {
        let (store, _tmp) = setup();
        let tables: Vec<String> = {
            let mut stmt = store
                .db
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(tables.contains(&"facts".to_string()));
        assert!(tables.contains(&"entities".to_string()));
        assert!(tables.contains(&"fact_entities".to_string()));
        assert!(tables.contains(&"episodes".to_string()));
        assert!(tables.contains(&"knowledge".to_string()));
        assert!(tables.contains(&"consolidation_log".to_string()));
        // FTS virtual tables
        assert!(tables.iter().any(|t| t.contains("facts_fts")));
        assert!(tables.iter().any(|t| t.contains("episodes_fts")));
        assert!(tables.iter().any(|t| t.contains("knowledge_fts")));
    }

    #[test]
    fn test_fts5_triggers_insert() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Rust is great", "tech", "lang", "", 0.5, "episodic", 0)
            .unwrap();
        let fts_count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM facts_fts WHERE rowid = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn test_fts5_triggers_delete() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Temp fact", "temp", "", "", 0.5, "episodic", 0)
            .unwrap();
        store.delete_fact(id).unwrap();
        let fts_count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM facts_fts WHERE rowid = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn test_fts5_triggers_update() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Original content here", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        // Update via direct SQL to test trigger
        store
            .db
            .execute(
                "UPDATE facts SET content = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
                rusqlite::params!("Updated content here", id),
            )
            .unwrap();
        // FTS should reflect new content
        let fts_count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM facts_fts WHERE facts_fts MATCH '\"updated*\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn test_indexes_exist() {
        let (store, _tmp) = setup();
        let indexes: Vec<String> = {
            let mut stmt = store
                .db
                .prepare("SELECT name FROM sqlite_master WHERE type='index' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(indexes.iter().any(|i| i.contains("idx_facts_trust")));
        assert!(indexes.iter().any(|i| i.contains("idx_facts_category")));
        assert!(indexes.iter().any(|i| i.contains("idx_facts_tier")));
        assert!(indexes.iter().any(|i| i.contains("idx_entities_name")));
        assert!(indexes.iter().any(|i| i.contains("idx_episodes_session")));
        assert!(indexes.iter().any(|i| i.contains("idx_episodes_outcome")));
        assert!(indexes.iter().any(|i| i.contains("idx_episodes_consolidated")));
        assert!(indexes.iter().any(|i| i.contains("idx_episodes_timestamp")));
        assert!(indexes.iter().any(|i| i.contains("idx_knowledge_topic")));
    }

    // ── Fact CRUD + Trust tests ─────────────────────────────────────────────

    #[test]
    fn test_add_fact_basic() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Alice works at Acme Corp", "people", "work", "", 0.5, "episodic", 0)
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_add_fact_dedup() {
        let (store, _tmp) = setup();
        let id1 = store
            .add_fact("Dup fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let id2 = store
            .add_fact("Dup fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_search_facts_basic() {
        let (store, _tmp) = setup();
        store
            .add_fact("Kuavo robot is a humanoid", "robot", "hw", "", 0.5, "episodic", 0)
            .unwrap();
        store
            .add_fact("Rust is a systems language", "tech", "lang", "", 0.5, "episodic", 0)
            .unwrap();

        let results = store.search_facts("Kuavo robot", None, 0.0, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Kuavo"));
    }

    #[test]
    fn test_search_facts_min_trust() {
        let (store, _tmp) = setup();
        store
            .add_fact("Low trust fact", "test", "", "", 0.1, "episodic", 0)
            .unwrap();
        store
            .add_fact("High trust fact", "test", "", "", 0.9, "episodic", 0)
            .unwrap();

        let results = store.search_facts("trust fact", None, 0.5, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("High"));
    }

    #[test]
    fn test_record_feedback_helpful() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Feedback test", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, true).unwrap();
        assert!((result.new_trust - 0.55).abs() < 1e-10);
        assert_eq!(result.helpful_count, 1);
    }

    #[test]
    fn test_record_feedback_unhelpful() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Unhelpful test", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, false).unwrap();
        assert!((result.new_trust - 0.40).abs() < 1e-10);
    }

    #[test]
    fn test_record_feedback_clamp() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Clamp test", "test", "", "", 0.98, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, true).unwrap();
        assert!((result.new_trust - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_retrieval_count_increments() {
        let (store, _tmp) = setup();
        store
            .add_fact("Retrieval counter test", "test", "", "", 0.5, "episodic", 0)
            .unwrap();

        store.search_facts("Retrieval counter", None, 0.0, 10).unwrap();
        store.search_facts("Retrieval counter", None, 0.0, 10).unwrap();

        let results = store.search_facts("Retrieval counter", None, 0.0, 10).unwrap();
        // retrieval_count was incremented 3 times (2 prior + 1 current search)
        // but the third search returns the value BEFORE its own increment,
        // so the fact's retrieval_count is 3 (updated in DB) but the returned
        // row was fetched before increment. Let's check via get_fact.
        let fact = store.get_fact(results[0].fact_id).unwrap().unwrap();
        assert_eq!(fact.retrieval_count, 3);
    }

    // ── Trust decay tests ───────────────────────────────────────────────────

    #[test]
    fn test_decay_stale() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Stale fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        // Force updated_at to be old
        store
            .db
            .execute(
                "UPDATE facts SET updated_at = datetime('now', '-10 days') WHERE fact_id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        let affected = store.decay_stale().unwrap();
        assert_eq!(affected, 1);
        let fact = store.get_fact(id).unwrap().unwrap();
        assert!((fact.trust_score - 0.498).abs() < 1e-10);
    }

    #[test]
    fn test_decay_recent_not_affected() {
        let (store, _tmp) = setup();
        store
            .add_fact("Recent fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let affected = store.decay_stale().unwrap();
        assert_eq!(affected, 0);
    }

    #[test]
    fn test_decay_clamp_floor() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Floor test", "test", "", "", 0.001, "episodic", 0)
            .unwrap();
        store
            .db
            .execute(
                "UPDATE facts SET updated_at = datetime('now', '-10 days') WHERE fact_id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        store.decay_stale().unwrap();
        let fact = store.get_fact(id).unwrap().unwrap();
        assert!((fact.trust_score).abs() < 1e-10);
    }

    // ── Entity tests ────────────────────────────────────────────────────────

    #[test]
    fn test_extract_entities_capitalized() {
        let entities = FactStore::extract_entities("Kuavo Robot is made by Acme Corp");
        assert!(entities.iter().any(|e| e.contains("Kuavo Robot")));
    }

    #[test]
    fn test_extract_entities_quoted() {
        let entities = FactStore::extract_entities(r#"The "special term" is important"#);
        assert!(entities.contains(&"special term".to_string()));
    }

    #[test]
    fn test_extract_entities_aka() {
        let entities = FactStore::extract_entities("Robert aka Bob went home");
        assert!(entities.iter().any(|e| e == "Robert"));
        assert!(entities.iter().any(|e| e == "Bob"));
    }

    #[test]
    fn test_resolve_entity_create_and_find() {
        let (store, _tmp) = setup();
        let id1 = store.resolve_entity("TestEntity").unwrap();
        let id2 = store.resolve_entity("TestEntity").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_entity_neighbors() {
        let (store, _tmp) = setup();
        let f1 = store
            .add_fact(
                "Alice works at Acme Corp",
                "people",
                "",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();
        let f2 = store
            .add_fact(
                "Bob works at Acme Corp",
                "people",
                "",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();

        // Get entity IDs
        let alice_id = store.resolve_entity("Alice").unwrap();
        let acme_id = store.resolve_entity("Acme Corp").unwrap();

        let neighbors = store.get_entity_neighbors(acme_id).unwrap();
        assert!(neighbors.iter().any(|n| n.entity_id == alice_id));
        assert!(neighbors.iter().all(|n| n.shared_facts >= 1));
    }

    // ── Episode tests ───────────────────────────────────────────────────────

    #[test]
    fn test_add_episode() {
        let (store, _tmp) = setup();
        let id = store
            .add_episode(
                "sess1",
                "deploy robot",
                "{}",
                "[]",
                "success",
                "deployed ok",
                0.8,
            )
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_search_episodes_fts() {
        let (store, _tmp) = setup();
        store
            .add_episode(
                "s1",
                "calibrate arm motors",
                "{}",
                "[]",
                "success",
                "",
                0.5,
            )
            .unwrap();
        store
            .add_episode(
                "s1",
                "deploy new firmware",
                "{}",
                "[]",
                "success",
                "",
                0.5,
            )
            .unwrap();

        let results = store.search_episodes("calibrate", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].task.contains("calibrate"));
    }

    #[test]
    fn test_get_unconsolidated() {
        let (store, _tmp) = setup();
        let id1 = store
            .add_episode("s1", "task a", "{}", "[]", "success", "", 0.5)
            .unwrap();
        let id2 = store
            .add_episode("s1", "task b", "{}", "[]", "failure", "", 0.7)
            .unwrap();

        let uncons = store.get_unconsolidated_episodes(10).unwrap();
        assert_eq!(uncons.len(), 2);

        store.mark_consolidated(&[id1]).unwrap();
        let uncons = store.get_unconsolidated_episodes(10).unwrap();
        assert_eq!(uncons.len(), 1);
        assert_eq!(uncons[0].episode_id, id2);
    }

    #[test]
    fn test_count_episodes() {
        let (store, _tmp) = setup();
        store
            .add_episode("s1", "a", "{}", "[]", "success", "", 0.5)
            .unwrap();
        store
            .add_episode("s1", "b", "{}", "[]", "failure", "", 0.5)
            .unwrap();
        store
            .add_episode("s1", "c", "{}", "[]", "success", "", 0.5)
            .unwrap();

        assert_eq!(store.count_episodes(None).unwrap(), 3);
        assert_eq!(store.count_episodes(Some("success")).unwrap(), 2);
        assert_eq!(store.count_episodes(Some("failure")).unwrap(), 1);
    }

    #[test]
    fn test_episode_outcome_constraint() {
        let (store, _tmp) = setup();
        let result = store.add_episode("s1", "bad", "{}", "[]", "invalid", "", 0.5);
        assert!(result.is_err());
    }

    // ── Knowledge tests ─────────────────────────────────────────────────────

    #[test]
    fn test_add_knowledge() {
        let (store, _tmp) = setup();
        let id = store
            .add_knowledge("ROS setup", "Install via apt", "[]", 0.9)
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_search_knowledge_fts() {
        let (store, _tmp) = setup();
        store
            .add_knowledge("ROS2 migration", "Steps to migrate from ROS1", "[]", 0.8)
            .unwrap();
        store
            .add_knowledge(
                "Docker basics",
                "Container fundamentals",
                "[]",
                0.7,
            )
            .unwrap();

        let results = store.search_knowledge("ROS2", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].topic.contains("ROS2"));
    }

    #[test]
    fn test_knowledge_access_count() {
        let (store, _tmp) = setup();
        store
            .add_knowledge("Access test", "Some content", "[]", 0.5)
            .unwrap();

        store.search_knowledge("Access", 10).unwrap();
        store.search_knowledge("Access", 10).unwrap();

        let results = store.search_knowledge("Access", 10).unwrap();
        // access_count incremented 3 times in DB, but the row returned in the
        // 3rd search was fetched before increment. Check via direct query.
        let count: i64 = store
            .db
            .query_row(
                "SELECT access_count FROM knowledge WHERE topic = 'Access test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_consolidation_log() {
        let (store, _tmp) = setup();
        assert!(store.get_last_consolidation().unwrap().is_none());

        store.log_consolidation(10, 3, "").unwrap();
        store.log_consolidation(5, 1, "some error").unwrap();

        let last = store.get_last_consolidation().unwrap().unwrap();
        assert_eq!(last.episodes_processed, 5);
        assert_eq!(last.knowledge_extracted, 1);
        assert_eq!(last.errors, "some error");
    }
}
