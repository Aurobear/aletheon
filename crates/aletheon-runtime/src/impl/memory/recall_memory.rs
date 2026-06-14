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
            "
        )?;
        Ok(Self { db })
    }

    /// Store a memory entry.
    pub fn store(&self, session_id: &str, entry_type: &str, content: &str, metadata: Option<&str>) -> anyhow::Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO recall_memory (timestamp, session_id, entry_type, content, metadata) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![now, session_id, entry_type, content, metadata],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Search by keyword (FTS-free for now).
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, timestamp, session_id, entry_type, content, metadata
             FROM recall_memory
             WHERE content LIKE ?1
             ORDER BY timestamp DESC
             LIMIT ?2"
        )?;

        let pattern = format!("%{}%", query);
        let entries = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
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
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Get recent entries.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, timestamp, session_id, entry_type, content, metadata
             FROM recall_memory
             ORDER BY timestamp DESC
             LIMIT ?1"
        )?;

        let entries = stmt.query_map([limit as i64], |row| {
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
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Count total entries.
    pub fn count(&self) -> anyhow::Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM recall_memory", [], |row| row.get(0)
        )?;
        Ok(count as usize)
    }
}
