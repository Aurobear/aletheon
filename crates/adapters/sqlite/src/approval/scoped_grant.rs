//! SQLite persistence for Application transient scoped approval grants.

use std::path::Path;
use std::sync::Mutex;

use application::approval::{ScopedApprovalGrantStore, ScopedGrantKey};
use rusqlite::{params, Connection};

pub struct SqliteScopedApprovalGrantStore {
    connection: Mutex<Connection>,
}

impl SqliteScopedApprovalGrantStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS transient_session_grants (
               principal_id TEXT NOT NULL, thread_id TEXT NOT NULL, tool TEXT NOT NULL,
               path_root TEXT NOT NULL DEFAULT '', subject_version INTEGER NOT NULL DEFAULT 0,
               subject_sha256 TEXT NOT NULL DEFAULT '', expires_at_ms INTEGER NOT NULL,
               PRIMARY KEY(principal_id,thread_id,tool,path_root,subject_version,subject_sha256)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl ScopedApprovalGrantStore for SqliteScopedApprovalGrantStore {
    fn clear(&self) -> anyhow::Result<()> {
        self.connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute("DELETE FROM transient_session_grants", [])?;
        Ok(())
    }

    fn grant(&self, key: &ScopedGrantKey, expires_at_ms: i64) -> anyhow::Result<()> {
        self.connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute(
                "INSERT OR REPLACE INTO transient_session_grants
                 (principal_id,thread_id,tool,path_root,subject_version,subject_sha256,expires_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    key.principal_id.0,
                    key.thread_id.0,
                    key.tool,
                    key.path_root,
                    key.subject_version,
                    key.subject_sha256,
                    expires_at_ms
                ],
            )?;
        Ok(())
    }

    fn has_tool_grant(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        Ok(self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT COUNT(*) > 0 FROM transient_session_grants
             WHERE principal_id=?1 AND thread_id=?2 AND tool=?3 AND path_root=''
               AND expires_at_ms>?4",
                params![principal_id.0, thread_id.0, tool, now_ms],
                |row| row.get(0),
            )?)
    }

    fn path_roots(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        subject_version: u32,
        subject_sha256: &str,
        now_ms: i64,
    ) -> anyhow::Result<Vec<String>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut statement = connection.prepare(
            "SELECT path_root FROM transient_session_grants WHERE principal_id=?1 AND thread_id=?2
             AND tool=?3 AND path_root<>'' AND subject_version=?4 AND subject_sha256=?5 AND expires_at_ms>?6",
        )?;
        let roots = statement
            .query_map(
                params![
                    principal_id.0,
                    thread_id.0,
                    tool,
                    subject_version,
                    subject_sha256,
                    now_ms
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(roots)
    }
}
