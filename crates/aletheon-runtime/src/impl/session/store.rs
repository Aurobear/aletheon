use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

/// SessionStore: query and manage sessions via SQLite.
pub struct SessionStore {
    db: Connection,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("sessions.db");
        let db = Connection::open(&db_path)?;

        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                last_active TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                message_count INTEGER DEFAULT 0
            );
            ",
        )?;

        Ok(Self { db })
    }

    pub fn create_session(&self, session_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT OR IGNORE INTO sessions (session_id, created_at, last_active) VALUES (?1, ?2, ?2)",
            rusqlite::params![session_id, now],
        )?;
        Ok(())
    }

    pub fn update_activity(&self, session_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.execute(
            "UPDATE sessions SET last_active = ?1 WHERE session_id = ?2",
            rusqlite::params![now, session_id],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT session_id FROM sessions ORDER BY last_active DESC")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }
}
