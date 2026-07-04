mod index;
mod query;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;

// ── Trust scoring constants ──────────────────────────────────────────────────
const HELPFUL_DELTA: f64 = 0.05;
const UNHELPFUL_DELTA: f64 = -0.10;
const TRUST_MIN: f64 = 0.0;
const TRUST_MAX: f64 = 1.0;
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
    // ── governance fields (indices 12..=16) ──
    pub scope: String,
    pub source: String,
    pub status: String,
    pub pinned: bool,
    pub subject: String,
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
    pub(crate) db: Connection,
}

impl FactStore {
    /// Open (or create) a FactStore at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening fact store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::create_schema(&db)?;
        Self::migrate_facts_table(&db)?;
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

    /// Idempotent migration: add governance columns if missing.
    /// Guards each ALTER TABLE with PRAGMA table_info so repeated opens are safe.
    fn migrate_facts_table(db: &Connection) -> Result<()> {
        let existing: Vec<String> = db
            .prepare("PRAGMA table_info(facts)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<std::result::Result<_, _>>()?;
        let add = |name: &str, ddl: &str| -> Result<()> {
            if !existing.iter().any(|c| c == name) {
                db.execute_batch(ddl)?;
            }
            Ok(())
        };
        add(
            "scope",
            "ALTER TABLE facts ADD COLUMN scope TEXT NOT NULL DEFAULT 'session';",
        )?;
        add(
            "source",
            "ALTER TABLE facts ADD COLUMN source TEXT NOT NULL DEFAULT 'conversation';",
        )?;
        add(
            "status",
            "ALTER TABLE facts ADD COLUMN status TEXT NOT NULL DEFAULT 'active';",
        )?;
        add(
            "pinned",
            "ALTER TABLE facts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;",
        )?;
        add(
            "subject",
            "ALTER TABLE facts ADD COLUMN subject TEXT NOT NULL DEFAULT '';",
        )?;
        db.execute_batch("CREATE INDEX IF NOT EXISTS idx_facts_scope ON facts(scope);")?;
        db.execute_batch("CREATE INDEX IF NOT EXISTS idx_facts_status ON facts(status);")?;
        Ok(())
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    pub(crate) fn map_fact_row(row: &rusqlite::Row) -> rusqlite::Result<FactRow> {
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
            scope: row.get(12)?,
            source: row.get(13)?,
            status: row.get(14)?,
            pinned: row.get::<_, i64>(15)? != 0,
            subject: row.get(16)?,
        })
    }
}

/// Check if content contains likely secrets (API keys, passwords, tokens).
pub(crate) fn is_sensitive(content: &str) -> bool {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)(sk-[a-z0-9]{16,}|api[_-]?key\s*[:=]\s*[a-z0-9_-]{8,}|password\s*[:=]\s*\S{4,}|bearer\s+[a-z0-9._-]{16,}|-----BEGIN [A-Z ]+PRIVATE KEY-----)",
        )
        .unwrap()
    });
    re.is_match(content)
}

/// Sanitize a user query for FTS5 MATCH.
pub(crate) fn sanitize_fts_query(query: &str) -> String {
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
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
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
        // FTS row should exist
        let fts_count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM facts_fts WHERE rowid = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn test_fts5_triggers_delete() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("To be deleted", "temp", "", "", 0.5, "episodic", 0)
            .unwrap();
        store.delete_fact(id).unwrap();
        let fts_count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM facts_fts WHERE rowid = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn test_fts5_triggers_update() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Original content", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        // Update content directly
        store
            .db
            .execute(
                "UPDATE facts SET content = ?1 WHERE fact_id = ?2",
                rusqlite::params!["Updated content", id],
            )
            .unwrap();
        // FTS should reflect update
        let results = store.search_facts("Updated", None, 0.0, 10).unwrap();
        assert!(results.iter().any(|f| f.fact_id == id));
    }

    #[test]
    fn test_indexes_exist() {
        let (store, _tmp) = setup();
        let indexes: Vec<String> = {
            let mut stmt = store
                .db
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(indexes.contains(&"idx_facts_trust".to_string()));
        assert!(indexes.contains(&"idx_facts_category".to_string()));
        assert!(indexes.contains(&"idx_facts_tier".to_string()));
        assert!(indexes.contains(&"idx_entities_name".to_string()));
        assert!(indexes.contains(&"idx_episodes_session".to_string()));
        assert!(indexes.contains(&"idx_episodes_outcome".to_string()));
        assert!(indexes.contains(&"idx_episodes_consolidated".to_string()));
        assert!(indexes.contains(&"idx_episodes_timestamp".to_string()));
        assert!(indexes.contains(&"idx_knowledge_topic".to_string()));
    }

    #[test]
    fn test_add_fact_basic() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact(
                "User prefers dark mode",
                "preference",
                "ui",
                "",
                0.7,
                "semantic",
                30,
            )
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_add_fact_dedup() {
        let (store, _tmp) = setup();
        let id1 = store
            .add_fact("Same fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let id2 = store
            .add_fact("Same fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_search_facts_basic() {
        let (store, _tmp) = setup();
        store
            .add_fact(
                "Rust is a systems language",
                "tech",
                "lang",
                "",
                0.8,
                "semantic",
                0,
            )
            .unwrap();
        store
            .add_fact(
                "Python is great for scripting",
                "tech",
                "lang",
                "",
                0.7,
                "semantic",
                0,
            )
            .unwrap();
        let results = store.search_facts("Rust", None, 0.0, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
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
        assert!(results[0].content.contains("High trust"));
    }

    #[test]
    fn test_record_feedback_helpful() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Good fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, true).unwrap();
        assert!(result.new_trust > result.old_trust);
        assert_eq!(result.helpful_count, 1);
    }

    #[test]
    fn test_record_feedback_unhelpful() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Bad fact", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, false).unwrap();
        assert!(result.new_trust < result.old_trust);
    }

    #[test]
    fn test_record_feedback_clamp() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("Edge case", "test", "", "", 0.0, "episodic", 0)
            .unwrap();
        let result = store.record_feedback(id, false).unwrap();
        assert!(result.new_trust >= 0.0);
    }

    #[test]
    fn test_retrieval_count_increments() {
        let (store, _tmp) = setup();
        store
            .add_fact("Searchable fact", "test", "", "", 0.8, "episodic", 0)
            .unwrap();
        let results1 = store.search_facts("Searchable", None, 0.0, 10).unwrap();
        let results2 = store.search_facts("Searchable", None, 0.0, 10).unwrap();
        assert!(results2[0].retrieval_count > results1[0].retrieval_count);
    }

    #[test]
    fn test_decay_stale() {
        let (store, _tmp) = setup();
        // Insert a fact and manually backdate it
        let id = store
            .add_fact("Old fact", "test", "", "", 0.8, "episodic", 0)
            .unwrap();
        store
            .db
            .execute(
                "UPDATE facts SET updated_at = datetime('now', '-14 days') WHERE fact_id = ?1",
                [id],
            )
            .unwrap();
        let affected = store.decay_stale().unwrap();
        assert_eq!(affected, 1);
        let fact = store.get_fact(id).unwrap().unwrap();
        assert!(fact.trust_score < 0.8);
    }

    #[test]
    fn test_decay_recent_not_affected() {
        let (store, _tmp) = setup();
        store
            .add_fact("Recent fact", "test", "", "", 0.8, "episodic", 0)
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
                "UPDATE facts SET updated_at = datetime('now', '-14 days') WHERE fact_id = ?1",
                [id],
            )
            .unwrap();
        store.decay_stale().unwrap();
        let fact = store.get_fact(id).unwrap().unwrap();
        assert!(fact.trust_score >= 0.0);
    }

    #[test]
    fn test_extract_entities_capitalized() {
        let entities = FactStore::extract_entities("I use Rust and Python daily");
        assert!(entities.contains(&"Rust".to_string()));
        assert!(entities.contains(&"Python".to_string()));
    }

    #[test]
    fn test_extract_entities_quoted() {
        let entities = FactStore::extract_entities(r#"He said "hello world" loudly"#);
        assert!(entities.contains(&"hello world".to_string()));
    }

    #[test]
    fn test_extract_entities_aka() {
        let entities = FactStore::extract_entities("JavaScript aka JS is popular");
        assert!(entities.contains(&"JavaScript".to_string()));
        assert!(entities.contains(&"JS".to_string()));
    }

    #[test]
    fn test_resolve_entity_create_and_find() {
        let (store, _tmp) = setup();
        let id1 = store.resolve_entity("Alice").unwrap();
        let id2 = store.resolve_entity("Alice").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_entity_neighbors() {
        let (store, _tmp) = setup();
        let _f1 = store
            .add_fact("Alice likes Rust", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let _f2 = store
            .add_fact("Bob likes Rust", "test", "", "", 0.5, "episodic", 0)
            .unwrap();
        let alice_id = store.resolve_entity("Alice").unwrap();
        let neighbors = store.get_entity_neighbors(alice_id).unwrap();
        assert!(!neighbors.is_empty());
    }

    #[test]
    fn test_add_episode() {
        let (store, _tmp) = setup();
        let id = store
            .add_episode(
                "session1",
                "Debug login issue",
                "{}",
                "[]",
                "success",
                "Fixed credentials",
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
                "Fix authentication bug",
                "{}",
                "[]",
                "success",
                "",
                0.5,
            )
            .unwrap();
        store
            .add_episode("s1", "Add dark mode", "{}", "[]", "success", "", 0.5)
            .unwrap();
        let results = store.search_episodes("authentication", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].task.contains("authentication"));
    }

    #[test]
    fn test_get_unconsolidated() {
        let (store, _tmp) = setup();
        store
            .add_episode("s1", "Task 1", "{}", "[]", "success", "", 0.5)
            .unwrap();
        store
            .add_episode("s1", "Task 2", "{}", "[]", "success", "", 0.3)
            .unwrap();
        let unconsolidated = store.get_unconsolidated_episodes(10).unwrap();
        assert_eq!(unconsolidated.len(), 2);
        // Higher importance first
        assert!(unconsolidated[0].importance >= unconsolidated[1].importance);
    }

    #[test]
    fn test_count_episodes() {
        let (store, _tmp) = setup();
        store
            .add_episode("s1", "T1", "{}", "[]", "success", "", 0.5)
            .unwrap();
        store
            .add_episode("s1", "T2", "{}", "[]", "failure", "", 0.5)
            .unwrap();
        assert_eq!(store.count_episodes(None).unwrap(), 2);
        assert_eq!(store.count_episodes(Some("success")).unwrap(), 1);
        assert_eq!(store.count_episodes(Some("failure")).unwrap(), 1);
    }

    #[test]
    fn test_episode_outcome_constraint() {
        let (store, _tmp) = setup();
        let result = store.add_episode("s1", "T", "{}", "[]", "invalid", "", 0.5);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_knowledge() {
        let (store, _tmp) = setup();
        let id = store
            .add_knowledge("Rust ownership", "Borrow checker enforces...", "[1,2]", 0.9)
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_search_knowledge_fts() {
        let (store, _tmp) = setup();
        store
            .add_knowledge(
                "Rust ownership",
                "The borrow checker enforces memory safety",
                "[]",
                0.9,
            )
            .unwrap();
        store
            .add_knowledge(
                "Python GIL",
                "Global Interpreter Lock limits concurrency",
                "[]",
                0.8,
            )
            .unwrap();
        let results = store.search_knowledge("ownership", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].topic.contains("Rust"));
    }

    #[test]
    fn test_knowledge_access_count() {
        let (store, _tmp) = setup();
        store
            .add_knowledge("Test topic", "Content", "[]", 0.5)
            .unwrap();
        let r1 = store.search_knowledge("topic", 10).unwrap();
        let r2 = store.search_knowledge("topic", 10).unwrap();
        assert!(r2[0].access_count > r1[0].access_count);
    }

    #[test]
    fn test_consolidation_log() {
        let (store, _tmp) = setup();
        store.log_consolidation(5, 2, "").unwrap();
        store.log_consolidation(3, 1, "timeout").unwrap();
        let last = store.get_last_consolidation().unwrap().unwrap();
        assert_eq!(last.episodes_processed, 3);
        assert_eq!(last.errors, "timeout");
    }

    #[test]
    fn migration_is_idempotent_and_adds_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrate.db");
        // Open twice -- migration must not error on second run
        {
            let _fs = FactStore::open(&path).unwrap();
        }
        let fs = FactStore::open(&path).unwrap();
        let mut stmt = fs.db.prepare("PRAGMA table_info(facts)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        for c in ["scope", "source", "status", "pinned", "subject"] {
            assert!(cols.contains(&c.to_string()), "missing column '{c}'");
        }
    }

    #[test]
    fn migration_preserves_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preserve.db");
        // Open, add a fact, close
        {
            let fs = FactStore::open(&path).unwrap();
            let id = fs
                .add_fact("preserved fact", "general", "", "", 0.5, "episodic", 0)
                .unwrap();
            assert!(id > 0);
        }
        // Re-open (triggers migration again), verify fact still there
        let fs = FactStore::open(&path).unwrap();
        let row = fs.get_fact(1).unwrap().unwrap();
        assert_eq!(row.content, "preserved fact");
    }

    #[test]
    fn get_fact_returns_governance_defaults() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("the sky is blue", "general", "", "", 0.5, "episodic", 0)
            .unwrap();
        let row = store.get_fact(id).unwrap().unwrap();
        assert_eq!(row.scope, "session");
        assert_eq!(row.source, "conversation");
        assert_eq!(row.status, "active");
        assert!(!row.pinned);
        assert_eq!(row.subject, "");
    }

    #[test]
    fn rejects_secrets_unless_explicit_source() {
        let (store, _tmp) = setup();
        let err = store.add_fact_governed(
            "my key is sk-abcdefghijklmnopqrstuvwx",
            "general",
            "",
            "project",
            "conversation",
            "",
            0.5,
            "episodic",
            0,
        );
        assert!(
            err.is_err(),
            "should reject secret from conversation source"
        );
        let ok = store.add_fact_governed(
            "my key is sk-abcdefghijklmnopqrstuvwx",
            "general",
            "",
            "project",
            "explicit",
            "",
            0.5,
            "episodic",
            0,
        );
        assert!(ok.is_ok(), "should allow secret from explicit source");
    }

    #[test]
    fn add_fact_governed_sets_fields_correctly() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact_governed(
                "rust is memory safe",
                "tech",
                "lang",
                "project",
                "explicit",
                "rust memory model",
                0.9,
                "semantic",
                30,
            )
            .unwrap();
        let row = store.get_fact(id).unwrap().unwrap();
        assert_eq!(row.content, "rust is memory safe");
        assert_eq!(row.scope, "project");
        assert_eq!(row.source, "explicit");
        assert_eq!(row.subject, "rust memory model");
        assert_eq!(row.tier, "semantic");
        assert_eq!(row.ttl_days, 30);
        assert!((row.trust_score - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn governed_search_excludes_archived() {
        let (store, _tmp) = setup();
        let keep = store
            .add_fact_governed(
                "rust is fast",
                "general",
                "",
                "project",
                "explicit",
                "",
                0.9,
                "semantic",
                0,
            )
            .unwrap();
        let arch = store
            .add_fact_governed(
                "rust is slow",
                "general",
                "",
                "project",
                "explicit",
                "",
                0.9,
                "semantic",
                0,
            )
            .unwrap();
        store.set_status(arch, "archived").unwrap();
        let hits = store
            .search_facts_governed("rust", Some("project"), false, 0.15, 10)
            .unwrap();
        let ids: Vec<i64> = hits.iter().map(|f| f.fact_id).collect();
        assert!(ids.contains(&keep));
        assert!(!ids.contains(&arch));
    }

    #[test]
    fn governed_search_respects_scope_filter() {
        let (store, _tmp) = setup();
        let s1 = store
            .add_fact_governed(
                "project fact xyz",
                "general",
                "",
                "project",
                "explicit",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();
        let s2 = store
            .add_fact_governed(
                "global fact xyz",
                "general",
                "",
                "global",
                "explicit",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();
        let hits = store
            .search_facts_governed("fact", Some("project"), false, 0.0, 10)
            .unwrap();
        let ids: Vec<i64> = hits.iter().map(|f| f.fact_id).collect();
        assert!(ids.contains(&s1));
        assert!(!ids.contains(&s2));
    }

    #[test]
    fn pin_and_list_roundtrip() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact_governed(
                "pin me", "general", "", "global", "explicit", "", 0.5, "semantic", 0,
            )
            .unwrap();
        assert!(store.set_pinned(id, true).unwrap());
        let all = store.list_facts(None, false, 50).unwrap();
        let pinned = all.iter().find(|f| f.fact_id == id).unwrap();
        assert!(pinned.pinned);
        assert!(store.set_pinned(id, false).unwrap());
        let all2 = store.list_facts(None, false, 50).unwrap();
        let unpinned = all2.iter().find(|f| f.fact_id == id).unwrap();
        assert!(!unpinned.pinned);
    }

    #[test]
    fn pinned_sorts_first_in_list() {
        let (store, _tmp) = setup();
        let _id1 = store
            .add_fact_governed(
                "first fact",
                "general",
                "",
                "global",
                "explicit",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();
        let id2 = store
            .add_fact_governed(
                "second fact",
                "general",
                "",
                "global",
                "explicit",
                "",
                0.5,
                "episodic",
                0,
            )
            .unwrap();
        store.set_pinned(id2, true).unwrap();
        let all = store.list_facts(None, false, 50).unwrap();
        assert_eq!(all[0].fact_id, id2, "pinned fact must sort first");
    }
}
