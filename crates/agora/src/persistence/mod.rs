//! Persistence adapter for Agora workspace commits.
//!
//! Agora workspaces are in-memory by default. The [`AgoraPersistence`] trait
//! allows plugging in a commit log so that committed operations survive process
//! restarts. [`InMemoryCommitLog`] is a process-lifetime store suitable for
//! testing and single-process use.

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::workspace::AgoraCommit;

/// Persistence backend for Agora commits.
///
/// Implementations are free to back the log with in-memory storage, a database,
/// or a file — the trait only requires append + recover-by-session.
#[async_trait]
pub trait AgoraPersistence: Send + Sync {
    /// Persist a committed operation, keyed by session id. Implementations must
    /// be idempotent by `(session, commit.id)` and reject conflicting payloads.
    async fn append_commit(&self, session: &str, commit: &AgoraCommit) -> Result<()>;

    /// Recover all commits for a session, in commit order.
    async fn recover(&self, session: &str) -> Result<Vec<AgoraCommit>>;

    /// Remove one session's durable working-state history.
    async fn clear_session(&self, session: &str) -> Result<()> {
        let _ = session;
        anyhow::bail!("Agora persistence backend does not support durable clear")
    }
}

/// Process-lifetime, in-memory commit log.
///
/// Stores a linear sequence of `(session_id, commit)` tuples behind a `Mutex`.
/// Survives across `AgoraRegistry` instances within the same process but is
/// lost on exit.
#[derive(Debug, Default)]
pub struct InMemoryCommitLog {
    entries: Mutex<Vec<(String, AgoraCommit)>>,
}

/// Durable append-only SQLite commit log used by the installed daemon.
pub struct SqliteAgoraPersistence {
    connection: Mutex<rusqlite::Connection>,
}

impl SqliteAgoraPersistence {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = rusqlite::Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS agora_commits (
                session_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                commit_id TEXT NOT NULL,
                checksum TEXT NOT NULL,
                commit_json TEXT NOT NULL,
                PRIMARY KEY(session_id, version),
                UNIQUE(session_id, commit_id)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[async_trait]
impl AgoraPersistence for SqliteAgoraPersistence {
    async fn append_commit(&self, session: &str, commit: &AgoraCommit) -> Result<()> {
        commit.validate_integrity()?;
        anyhow::ensure!(
            commit.space.0 == session,
            "commit persistence space mismatch"
        );
        let encoded = serde_json::to_string(commit)?;
        let connection = self.connection.lock().await;
        let existing = connection.query_row(
            "SELECT commit_json FROM agora_commits WHERE session_id=?1 AND commit_id=?2",
            rusqlite::params![session, commit.id.to_string()],
            |row| row.get::<_, String>(0),
        );
        match existing {
            Ok(existing) => {
                anyhow::ensure!(existing == encoded, "workspace commit id collision");
                return Ok(());
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(error) => return Err(error.into()),
        }
        connection.execute(
            "INSERT INTO agora_commits(session_id, version, commit_id, checksum, commit_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                session,
                commit.version,
                commit.id.to_string(),
                commit.checksum,
                encoded
            ],
        )?;
        Ok(())
    }

    async fn recover(&self, session: &str) -> Result<Vec<AgoraCommit>> {
        let connection = self.connection.lock().await;
        let mut statement = connection.prepare(
            "SELECT commit_json FROM agora_commits WHERE session_id=?1 ORDER BY version ASC",
        )?;
        let rows = statement.query_map([session], |row| row.get::<_, String>(0))?;
        let mut commits = Vec::new();
        for row in rows {
            let commit: AgoraCommit = serde_json::from_str(&row?)?;
            commit.validate_integrity()?;
            anyhow::ensure!(commit.space.0 == session, "recovered commit space mismatch");
            anyhow::ensure!(
                commit.version == commits.len() as u64 + 1,
                "recovered Agora history is not contiguous"
            );
            commits.push(commit);
        }
        Ok(commits)
    }

    async fn clear_session(&self, session: &str) -> Result<()> {
        self.connection
            .lock()
            .await
            .execute("DELETE FROM agora_commits WHERE session_id=?1", [session])?;
        Ok(())
    }
}

impl InMemoryCommitLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AgoraPersistence for InMemoryCommitLog {
    async fn append_commit(&self, session: &str, commit: &AgoraCommit) -> Result<()> {
        let mut entries = self.entries.lock().await;
        if let Some((_, existing)) = entries
            .iter()
            .find(|(candidate, existing)| candidate == session && existing.id == commit.id)
        {
            anyhow::ensure!(
                serde_json::to_vec(existing)? == serde_json::to_vec(commit)?,
                "workspace commit id collision"
            );
            return Ok(());
        }
        entries.push((session.to_string(), commit.clone()));
        Ok(())
    }

    async fn recover(&self, session: &str) -> Result<Vec<AgoraCommit>> {
        let entries = self.entries.lock().await;
        Ok(entries
            .iter()
            .filter(|(s, _)| s == session)
            .map(|(_, c)| c.clone())
            .collect())
    }

    async fn clear_session(&self, session: &str) -> Result<()> {
        self.entries
            .lock()
            .await
            .retain(|(candidate, _)| candidate != session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::AgoraOperation;
    use serde_json::json;
    use uuid::Uuid;

    fn commit(id: Uuid, version: u64, key: &str, value: serde_json::Value, at: i64) -> AgoraCommit {
        let proposal = crate::AgoraProposal {
            id,
            space: ::contracts::AgoraSpaceId("s".into()),
            author: ::contracts::ProcessId(uuid::Uuid::from_u128(3)),
            base_version: version - 1,
            operation: AgoraOperation::PublishFact {
                key: key.into(),
                value,
            },
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        };
        AgoraCommit::from_proposal(&proposal, version, at, None).unwrap()
    }

    #[tokio::test]
    async fn append_then_recover_single_session() {
        let log = InMemoryCommitLog::new();

        let c1 = commit(Uuid::new_v4(), 1, "x", json!(1), 1000);
        let c2 = commit(Uuid::new_v4(), 2, "y", json!(2), 1001);

        log.append_commit("s1", &c1).await.unwrap();
        log.append_commit("s1", &c2).await.unwrap();

        let recovered = log.recover("s1").await.unwrap();
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].id, c1.id);
        assert_eq!(recovered[1].id, c2.id);
    }

    #[tokio::test]
    async fn append_then_recover_multi_session() {
        let log = InMemoryCommitLog::new();

        let c1 = commit(Uuid::new_v4(), 1, "a", json!(1), 1000);
        let c2 = commit(Uuid::new_v4(), 2, "b", json!(2), 1001);

        log.append_commit("s1", &c1).await.unwrap();
        log.append_commit("s2", &c2).await.unwrap();

        let s1_recovered = log.recover("s1").await.unwrap();
        assert_eq!(s1_recovered.len(), 1);
        assert_eq!(s1_recovered[0].id, c1.id);

        let s2_recovered = log.recover("s2").await.unwrap();
        assert_eq!(s2_recovered.len(), 1);
        assert_eq!(s2_recovered[0].id, c2.id);
    }

    #[tokio::test]
    async fn recover_unknown_session_is_empty() {
        let log = InMemoryCommitLog::new();
        let recovered = log.recover("nope").await.unwrap();
        assert!(recovered.is_empty());
    }

    #[tokio::test]
    async fn attention_commit_payload_roundtrips_unchanged() {
        let log = InMemoryCommitLog::new();
        let proposal = crate::AgoraProposal {
            id: Uuid::new_v4(),
            space: ::contracts::AgoraSpaceId("s".into()),
            author: ::contracts::ProcessId(uuid::Uuid::from_u128(3)),
            base_version: 0,
            operation: AgoraOperation::UpdateAttention {
                focus: Some("a".into()),
                priorities: vec!["a".into(), "b".into()],
                selection_ref: "selection:1".into(),
            },
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        };
        let commit = AgoraCommit::from_proposal(&proposal, 1, 1000, None).unwrap();
        log.append_commit("s", &commit).await.unwrap();
        let recovered = log.recover("s").await.unwrap();
        assert_eq!(
            serde_json::to_value(&recovered[0]).unwrap(),
            serde_json::to_value(commit).unwrap()
        );
    }

    #[tokio::test]
    async fn sqlite_commit_log_survives_reopen_and_rejects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agora.db");
        let c1 = commit(Uuid::new_v4(), 1, "durable", json!(true), 1000);
        let store = SqliteAgoraPersistence::open(&path).unwrap();
        store.append_commit("s", &c1).await.unwrap();
        drop(store);

        let reopened = SqliteAgoraPersistence::open(&path).unwrap();
        let recovered = reopened.recover("s").await.unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(
            serde_json::to_value(&recovered[0]).unwrap(),
            serde_json::to_value(&c1).unwrap()
        );
        reopened
            .connection
            .lock()
            .await
            .execute(
                "UPDATE agora_commits SET commit_json=?1 WHERE session_id='s' AND version=1",
                ["{}"],
            )
            .unwrap();
        assert!(reopened.recover("s").await.is_err());
    }
}
