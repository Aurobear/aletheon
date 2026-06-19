use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Memory entry stored in Recall Memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub entry_type: String,
    pub content: String,
    pub metadata: Option<String>,
}

/// L2 Recall Memory -- SQLite-backed conversation history and tool call records.
pub struct RecallMemory {
    db: Connection,
}

impl RecallMemory {
    pub fn new(db_path: &std::path::Path) -> anyhow::Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS recall_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                session_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_recall_session ON recall_memory(session_id);
            CREATE INDEX IF NOT EXISTS idx_recall_type ON recall_memory(entry_type);
            CREATE INDEX IF NOT EXISTS idx_recall_time ON recall_memory(timestamp);
            ",
        )?;

        // FTS5 virtual table for full-text search with BM25 ranking
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS recall_memory_fts USING fts5(
                content,
                content=recall_memory,
                content_rowid=id,
                tokenize='porter unicode61'
            );",
        )?;

        // Triggers to keep FTS in sync with the main table
        db.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS recall_ai AFTER INSERT ON recall_memory BEGIN
                INSERT INTO recall_memory_fts(rowid, content) VALUES (new.id, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS recall_ad AFTER DELETE ON recall_memory BEGIN
                INSERT INTO recall_memory_fts(recall_memory_fts, rowid, content) VALUES('delete', old.id, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS recall_au AFTER UPDATE ON recall_memory BEGIN
                INSERT INTO recall_memory_fts(recall_memory_fts, rowid, content) VALUES('delete', old.id, old.content);
                INSERT INTO recall_memory_fts(rowid, content) VALUES (new.id, new.content);
            END;"
        )?;

        Ok(Self { db })
    }

    /// Store a memory entry.
    pub fn store(
        &self,
        session_id: &str,
        entry_type: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> anyhow::Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO recall_memory (timestamp, session_id, entry_type, content, metadata) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![now, session_id, entry_type, content, metadata],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Search by keyword using FTS5 with BM25 ranking.
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<MemoryEntry>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = sanitize_fts_query(query);

        let mut stmt = self.db.prepare(
            "SELECT r.id, r.timestamp, r.session_id, r.entry_type, r.content, r.metadata
             FROM recall_memory r
             INNER JOIN recall_memory_fts fts ON r.id = fts.rowid
             WHERE recall_memory_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let entries = stmt
            .query_map(rusqlite::params![fts_query, limit as i64], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Utc),
                    session_id: row.get(2)?,
                    entry_type: row.get(3)?,
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Fallback to LIKE search if FTS returns no results
        // (e.g. query contains only stopwords)
        if entries.is_empty() {
            return self.search_like(query, limit);
        }

        Ok(entries)
    }

    /// Fallback LIKE-based search (kept for edge cases).
    fn search_like(&self, query: &str, limit: usize) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, timestamp, session_id, entry_type, content, metadata
             FROM recall_memory
             WHERE content LIKE ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;

        let pattern = format!("%{}%", query);
        let entries = stmt
            .query_map(rusqlite::params![pattern, limit as i64], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Utc),
                    session_id: row.get(2)?,
                    entry_type: row.get(3)?,
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Get recent entries.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, timestamp, session_id, entry_type, content, metadata
             FROM recall_memory
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;

        let entries = stmt
            .query_map([limit as i64], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Utc),
                    session_id: row.get(2)?,
                    entry_type: row.get(3)?,
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Count total entries.
    pub fn count(&self) -> anyhow::Result<usize> {
        let count: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM recall_memory", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Rebuild the FTS index from existing data.
    /// Call this once after upgrading from the LIKE-based search.
    pub fn migrate_to_fts(&self) -> anyhow::Result<usize> {
        let count: usize = self
            .db
            .query_row("SELECT COUNT(*) FROM recall_memory", [], |row| row.get(0))?;
        self.db.execute_batch(
            "INSERT INTO recall_memory_fts(rowid, content)
             SELECT id, content FROM recall_memory",
        )?;
        Ok(count)
    }
}

/// Sanitize a user query for FTS5 MATCH.
/// Wraps each word in quotes to prevent FTS5 syntax errors,
/// and adds prefix matching for partial word matches.
fn sanitize_fts_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove FTS5 special characters and wrap in quotes
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                return String::new();
            }
            // Use prefix matching: "word*" matches word, words, workflow, etc.
            format!("\"{}*\"", clean)
        })
        .filter(|s| !s.is_empty())
        .collect();

    if words.is_empty() {
        // Return a safe no-match query
        return "\"\"".to_string();
    }

    // OR between words for broader matching
    words.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup_recall() -> (RecallMemory, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let recall = RecallMemory::new(tmp.path()).unwrap();
        (recall, tmp)
    }

    #[test]
    fn fts_search_finds_content() {
        let (recall, _tmp) = setup_recall();
        recall
            .store(
                "s1",
                "user",
                "The quick brown fox jumps over the lazy dog",
                None,
            )
            .unwrap();
        recall
            .store(
                "s1",
                "user",
                "I prefer using Rust for systems programming",
                None,
            )
            .unwrap();

        let results = recall.search("fox", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("fox"));
    }

    #[test]
    fn fts_search_partial_match() {
        let (recall, _tmp) = setup_recall();
        recall
            .store("s1", "user", "Configuration management is important", None)
            .unwrap();

        let results = recall.search("config", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn fts_search_multiple_results() {
        let (recall, _tmp) = setup_recall();
        recall
            .store("s1", "user", "Rust is a systems language", None)
            .unwrap();
        recall
            .store("s1", "user", "I love Rust programming", None)
            .unwrap();
        recall
            .store("s1", "user", "Python is also nice", None)
            .unwrap();

        let results = recall.search("Rust", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn fts_search_empty_query() {
        let (recall, _tmp) = setup_recall();
        recall.store("s1", "user", "hello world", None).unwrap();

        let results = recall.search("", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fts_search_no_match() {
        let (recall, _tmp) = setup_recall();
        recall.store("s1", "user", "hello world", None).unwrap();

        let results = recall.search("xyznonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn sanitize_handles_special_chars() {
        let q = sanitize_fts_query("hello AND OR NOT");
        // The sanitizer wraps each word in quotes with prefix matching.
        // "AND", "OR", "NOT" become "AND*", "OR*", "NOT*" which FTS5 treats
        // as literal tokens (not operators) when inside quotes.
        assert!(q.contains("\"AND*\""));
        assert!(q.contains("\"OR*\""));
        assert!(q.contains("\"NOT*\""));
    }

    #[test]
    fn sanitize_empty_input() {
        let q = sanitize_fts_query("   ");
        assert_eq!(q, "\"\"");
    }

    #[test]
    fn migrate_to_fts_backfills() {
        let (recall, _tmp) = setup_recall();
        recall
            .store("s1", "user", "alpha beta gamma", None)
            .unwrap();
        recall
            .store("s1", "user", "delta epsilon zeta", None)
            .unwrap();

        let count = recall.migrate_to_fts().unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn fts_ranking_prefers_relevant() {
        let (recall, _tmp) = setup_recall();
        recall.store("s1", "user", "Rust", None).unwrap();
        recall
            .store(
                "s1",
                "user",
                "I use Rust for everything, Rust is great, Rust Rust Rust",
                None,
            )
            .unwrap();

        let results = recall.search("Rust", 10).unwrap();
        assert_eq!(results.len(), 2);
        // The more relevant document should rank first (BM25)
        assert!(results[0].content.contains("Rust for everything"));
    }
}
