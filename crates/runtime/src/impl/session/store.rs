//! Session store with messages, metadata, fork, and archive support.
//!
//! Backed by SQLite. Schema includes `messages_json` and `metadata_json` columns
//! for full session persistence.

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

/// A full session record from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    pub messages_json: String,
    pub metadata_json: String,
}

/// SessionStore: query and manage sessions via SQLite.
pub struct SessionStore {
    db: Connection,
}

impl SessionStore {
    /// Open (or create) a session database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                status TEXT NOT NULL DEFAULT 'active',
                messages_json TEXT NOT NULL DEFAULT '[]',
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
            ",
        )?;
        Ok(Self { db })
    }

    /// Convenience: open a session store in a data directory (creates sessions.db).
    pub fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("sessions.db");
        Self::open(&db_path)
    }

    /// Create a session with default empty fields (legacy compat).
    pub fn create_session(&self, session_id: &str) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO sessions (session_id, messages_json, metadata_json) VALUES (?1, '[]', '{}')",
            rusqlite::params![session_id],
        )?;
        Ok(())
    }

    /// Update last_active timestamp (legacy compat).
    pub fn update_activity(&self, session_id: &str) -> Result<()> {
        self.db.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        Ok(())
    }

    /// Get the most recent session id (legacy compat).
    pub fn most_recent(&self) -> Result<Option<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT session_id FROM sessions ORDER BY updated_at DESC LIMIT 1")?;
        let mut rows = stmt.query([])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }

    /// List all session ids (legacy compat).
    pub fn list_sessions(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT session_id FROM sessions ORDER BY updated_at DESC")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Save (INSERT OR REPLACE) a session with full data.
    pub fn save(&self, session_id: &str, messages_json: &str, metadata_json: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO sessions (session_id, messages_json, metadata_json, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                messages_json = excluded.messages_json,
                metadata_json = excluded.metadata_json,
                updated_at = datetime('now')",
            rusqlite::params![session_id, messages_json, metadata_json],
        )?;
        Ok(())
    }

    /// Load a session by id. Returns `None` if not found.
    pub fn load(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        let mut stmt = self.db.prepare(
            "SELECT session_id, created_at, updated_at, status, messages_json, metadata_json
             FROM sessions WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(SessionRecord {
                session_id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                status: row.get(3)?,
                messages_json: row.get(4)?,
                metadata_json: row.get(5)?,
            })),
            None => Ok(None),
        }
    }

    /// List sessions, optionally filtered by status, with a limit.
    pub fn list(&self, status: Option<&str>, limit: usize) -> Result<Vec<SessionRecord>> {
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status {
            Some(s) => (
                "SELECT session_id, created_at, updated_at, status, messages_json, metadata_json
                 FROM sessions WHERE status = ?1 ORDER BY updated_at DESC LIMIT ?2",
                vec![
                    Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit as i64),
                ],
            ),
            None => (
                "SELECT session_id, created_at, updated_at, status, messages_json, metadata_json
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
                vec![Box::new(limit as i64) as Box<dyn rusqlite::types::ToSql>],
            ),
        };
        let mut stmt = self.db.prepare(sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let records = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(SessionRecord {
                    session_id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    status: row.get(3)?,
                    messages_json: row.get(4)?,
                    metadata_json: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// Fork a session: copy messages from source into a new session with `new_id`.
    pub fn fork(&self, source_id: &str, new_id: &str) -> Result<SessionRecord> {
        let source = self
            .load(source_id)?
            .ok_or_else(|| anyhow::anyhow!("source session not found: {}", source_id))?;
        self.save(new_id, &source.messages_json, &source.metadata_json)?;
        // Return the newly created record
        Ok(self.load(new_id)?.expect("just inserted; must exist"))
    }

    /// Archive a session (set status to "archived").
    pub fn archive(&self, session_id: &str) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE sessions SET status = 'archived', updated_at = datetime('now') WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        if changed == 0 {
            anyhow::bail!("session not found: {}", session_id);
        }
        Ok(())
    }

    /// Delete a session. Returns `true` if a row was deleted.
    pub fn delete(&self, session_id: &str) -> Result<bool> {
        let changed = self.db.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        Ok(changed > 0)
    }

    /// Auto-save: convenience that calls `save` with status="active".
    pub fn auto_save(&self, session_id: &str, messages_json: &str) -> Result<()> {
        // Ensure the session exists with active status, then update messages.
        self.db.execute(
            "INSERT INTO sessions (session_id, status, messages_json, metadata_json, updated_at)
             VALUES (?1, 'active', ?2, '{}', datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                messages_json = excluded.messages_json,
                status = 'active',
                updated_at = datetime('now')",
            rusqlite::params![session_id, messages_json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        store
            .save(
                "s1",
                r#"[{"role":"user","content":"hi"}]"#,
                r#"{"key":"val"}"#,
            )
            .unwrap();
        let rec = store.load("s1").unwrap().expect("should exist");

        assert_eq!(rec.session_id, "s1");
        assert_eq!(rec.messages_json, r#"[{"role":"user","content":"hi"}]"#);
        assert_eq!(rec.metadata_json, r#"{"key":"val"}"#);
        assert_eq!(rec.status, "active");
    }

    #[test]
    fn list_with_status_filter() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        store.save("s1", "[]", "{}").unwrap();
        store.save("s2", "[]", "{}").unwrap();
        store.archive("s2").unwrap();

        let active = store.list(Some("active"), 100).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "s1");

        let archived = store.list(Some("archived"), 100).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].session_id, "s2");

        let all = store.list(None, 100).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn fork_copies_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        store.save("src", r#"[{"msg":1}]"#, r#"{"a":1}"#).unwrap();
        let forked = store.fork("src", "dst").unwrap();

        assert_eq!(forked.session_id, "dst");
        assert_eq!(forked.messages_json, r#"[{"msg":1}]"#);
        assert_eq!(forked.metadata_json, r#"{"a":1}"#);

        // Source is untouched
        let src = store.load("src").unwrap().unwrap();
        assert_eq!(src.messages_json, r#"[{"msg":1}]"#);
    }

    #[test]
    fn archive_sets_status() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        store.save("s1", "[]", "{}").unwrap();
        store.archive("s1").unwrap();

        let rec = store.load("s1").unwrap().unwrap();
        assert_eq!(rec.status, "archived");
    }

    #[test]
    fn delete_removes_session() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        store.save("s1", "[]", "{}").unwrap();
        assert!(store.delete("s1").unwrap());
        assert!(store.load("s1").unwrap().is_none());

        // Deleting non-existent returns false
        assert!(!store.delete("nope").unwrap());
    }

    #[test]
    fn auto_save_creates_and_updates() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        // Auto-save creates a new session
        store.auto_save("s1", r#"[1]"#).unwrap();
        let rec = store.load("s1").unwrap().unwrap();
        assert_eq!(rec.messages_json, r#"[1]"#);
        assert_eq!(rec.status, "active");

        // Auto-save updates existing
        store.auto_save("s1", r#"[1,2]"#).unwrap();
        let rec = store.load("s1").unwrap().unwrap();
        assert_eq!(rec.messages_json, r#"[1,2]"#);
        assert_eq!(rec.status, "active");
    }

    #[test]
    fn fork_source_not_found_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();

        let result = store.fork("nonexistent", "new");
        assert!(result.is_err());
    }

    #[test]
    fn most_recent_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        assert_eq!(store.most_recent().unwrap(), None);
    }

    #[test]
    fn create_session_legacy_compat() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        store.create_session("s1").unwrap();
        let rec = store.load("s1").unwrap().unwrap();
        assert_eq!(rec.messages_json, "[]");
        assert_eq!(rec.metadata_json, "{}");
    }
}
