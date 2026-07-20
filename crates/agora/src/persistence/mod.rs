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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::AgoraOperation;
    use serde_json::json;
    use uuid::Uuid;

    fn commit(id: Uuid, version: u64, key: &str, value: serde_json::Value, at: i64) -> AgoraCommit {
        let proposal = fabric::AgoraProposal {
            id,
            space: fabric::AgoraSpaceId("s".into()),
            author: fabric::ProcessId(uuid::Uuid::from_u128(3)),
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
        let proposal = fabric::AgoraProposal {
            id: Uuid::new_v4(),
            space: fabric::AgoraSpaceId("s".into()),
            author: fabric::ProcessId(uuid::Uuid::from_u128(3)),
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
}
