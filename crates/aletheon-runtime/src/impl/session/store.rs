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

    pub fn most_recent(&self) -> Result<Option<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT session_id FROM sessions ORDER BY last_active DESC LIMIT 1")?;
        let mut rows = stmt.query([])?;
        Ok(rows
            .next()?
            .map(|r| r.get::<_, String>(0))
            .transpose()?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        assert_eq!(store.most_recent().unwrap(), None);
    }

    #[test]
    fn most_recent_returns_latest() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        store.create_session("s1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.create_session("s2").unwrap();
        assert_eq!(store.most_recent().unwrap(), Some("s2".to_string()));
    }

    #[test]
    fn most_recent_after_update_activity() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        store.create_session("s1").unwrap();
        store.create_session("s2").unwrap();
        // s1 was created first, but update its activity to make it most recent
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.update_activity("s1").unwrap();
        assert_eq!(store.most_recent().unwrap(), Some("s1".to_string()));
    }
}
