//! AgoraRegistry — manages per-session Workspaces and implements AgoraOps.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use fabric::include::agora::{AgoraView, AgoraViewRequest, CommitReceipt, WorkspaceCommitPermit};
use fabric::{AgoraOps, ProcessId};

use crate::persistence::AgoraPersistence;
use crate::workspace::{AgoraCommit, AgoraOperation, AgoraProposal, Workspace};

/// Owns one `Workspace` per session id. Cheap to clone via `Arc`.
pub struct AgoraRegistry {
    sessions: Mutex<HashMap<String, Arc<SpaceSlot>>>,
    proposal_index: Mutex<HashMap<uuid::Uuid, String>>,
    persistence: Option<Arc<dyn AgoraPersistence>>,
    clock: Arc<dyn fabric::Clock>,
}

struct SpaceSlot {
    workspace: Mutex<Workspace>,
    commit_gate: Mutex<()>,
}

impl SpaceSlot {
    fn new(session: &str, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            workspace: Mutex::new(Workspace::new(session, clock)),
            commit_gate: Mutex::new(()),
        }
    }
}

impl AgoraRegistry {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            proposal_index: Mutex::new(HashMap::new()),
            persistence: None,
            clock,
        }
    }

    /// Create a registry backed by a persistence adapter.
    ///
    /// Every `commit()` will also persist to the adapter. Call
    /// [`recover_session`](Self::recover_session) to replay persisted commits
    /// into a workspace after a restart.
    pub fn new_with_persistence(
        persistence: Arc<dyn AgoraPersistence>,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            proposal_index: Mutex::new(HashMap::new()),
            persistence: Some(persistence),
            clock,
        }
    }

    async fn space(&self, session: &str) -> Arc<SpaceSlot> {
        let mut sessions = self.sessions.lock().await;
        sessions
            .entry(session.to_string())
            .or_insert_with(|| Arc::new(SpaceSlot::new(session, self.clock.clone())))
            .clone()
    }

    async fn existing_space(&self, session: &str) -> Option<Arc<SpaceSlot>> {
        self.sessions.lock().await.get(session).cloned()
    }

    async fn commit_transaction(
        &self,
        session: &str,
        proposal_id: uuid::Uuid,
        permit: Option<&WorkspaceCommitPermit>,
    ) -> Result<AgoraCommit, String> {
        let slot = self.space(session).await;
        let _gate = slot.commit_gate.lock().await;
        let prepared = {
            let workspace = slot.workspace.lock().await;
            workspace
                .prepare_commit(proposal_id, permit)
                .map_err(|error| error.to_string())?
        };

        if let Some(persistence) = &self.persistence {
            persistence
                .append_commit(session, &prepared)
                .await
                .map_err(|error| format!("persistence write failed: {error}"))?;
        }

        {
            let mut workspace = slot.workspace.lock().await;
            workspace
                .apply_prepared_commit(prepared.clone())
                .map_err(|error| error.to_string())?;
        }
        self.proposal_index.lock().await.remove(&proposal_id);
        Ok(prepared)
    }

    /// Replay persisted commits for `session` into the workspace.
    ///
    /// After calling this, the workspace version will reflect all previously
    /// committed operations. Returns the number of commits replayed.
    pub async fn recover_session(&self, session: &str) -> Result<usize> {
        let persistence = match &self.persistence {
            Some(p) => p,
            None => return Ok(0),
        };

        let commits = persistence.recover(session).await?;
        let count = commits.len();
        if count == 0 {
            return Ok(0);
        }

        let slot = self.space(session).await;
        let _gate = slot.commit_gate.lock().await;
        let mut workspace = slot.workspace.lock().await;
        let mut recovered = workspace.clone();
        let mut replayed = 0;
        for commit in commits {
            if recovered.apply_commit(commit)? {
                replayed += 1;
            }
        }
        *workspace = recovered;

        Ok(replayed)
    }

    /// Write a value onto a session's blackboard.
    #[deprecated(note = "Use AgoraService propose/commit; publish is backend compatibility only")]
    pub async fn publish(&self, session: &str, key: &str, value: Value) -> Result<()> {
        let slot = self.space(session).await;
        let mut ws = slot.workspace.lock().await;
        ws.blackboard.set(key, value);
        Ok(())
    }

    /// Merge a JSON patch into the session workspace.
    #[deprecated(note = "Use AgoraService propose/commit; update is backend compatibility only")]
    pub async fn update(&self, session: &str, patch: Value) -> Result<()> {
        let slot = self.space(session).await;
        let mut ws = slot.workspace.lock().await;
        ws.blackboard.merge(patch);
        Ok(())
    }
}

#[async_trait]
impl AgoraOps for AgoraRegistry {
    async fn recall(&self, session: &str, key: &str) -> Result<Option<Value>> {
        let Some(slot) = self.existing_space(session).await else {
            return Ok(None);
        };
        let workspace = slot.workspace.lock().await;
        Ok(workspace.blackboard.get(key).cloned())
    }

    async fn snapshot(&self, session: &str) -> Result<Value> {
        let Some(slot) = self.existing_space(session).await else {
            return Ok(Value::Null);
        };
        let snapshot = slot.workspace.lock().await.snapshot();
        Ok(snapshot)
    }

    async fn version(&self, session: &str) -> Result<u64> {
        let Some(slot) = self.existing_space(session).await else {
            return Ok(0);
        };
        let version = slot.workspace.lock().await.version;
        Ok(version)
    }

    async fn clear(&self, session: &str) -> Result<()> {
        if let Some(slot) = self.existing_space(session).await {
            let _gate = slot.commit_gate.lock().await;
            let mut workspace = slot.workspace.lock().await;
            let ids: Vec<_> = workspace.proposals.keys().copied().collect();
            workspace.clear();
            let mut index = self.proposal_index.lock().await;
            for id in ids {
                index.remove(&id);
            }
        }
        Ok(())
    }

    async fn trace(&self, session: &str, kind: &str, content: Value) -> Result<()> {
        let slot = self.space(session).await;
        slot.workspace.lock().await.trace.push(kind, content);
        Ok(())
    }

    async fn propose(
        &self,
        session: &str,
        base_version: u64,
        operation: AgoraOperation,
        author: ProcessId,
    ) -> Result<AgoraProposal, String> {
        let slot = self.space(session).await;
        let proposal = slot
            .workspace
            .lock()
            .await
            .propose(base_version, operation, author)
            .map_err(|c| {
                format!(
                    "version conflict: expected {}, actual {}",
                    c.expected, c.actual
                )
            })?;
        self.proposal_index
            .lock()
            .await
            .insert(proposal.id, session.to_string());
        Ok(proposal)
    }

    async fn commit(&self, session: &str, proposal_id: uuid::Uuid) -> Result<AgoraCommit, String> {
        self.commit_transaction(session, proposal_id, None).await
    }

    async fn commit_with_permit(
        &self,
        session: &str,
        proposal_id: uuid::Uuid,
        permit: WorkspaceCommitPermit,
    ) -> Result<AgoraCommit, String> {
        if permit.space.0 != session || permit.proposal_id != proposal_id {
            return Err("commit permit does not address requested transaction".into());
        }
        self.commit_transaction(session, proposal_id, Some(&permit))
            .await
    }

    async fn reject(
        &self,
        session: &str,
        proposal_id: uuid::Uuid,
        reason: fabric::RejectReason,
    ) -> Result<(), String> {
        let slot = self.space(session).await;
        let _gate = slot.commit_gate.lock().await;
        slot.workspace
            .lock()
            .await
            .reject(proposal_id, reason)
            .ok_or_else(|| format!("proposal {} not found in session {}", proposal_id, session))?;
        self.proposal_index.lock().await.remove(&proposal_id);
        Ok(())
    }

    async fn changes_since(&self, session: &str, since_version: u64) -> Vec<AgoraCommit> {
        let Some(slot) = self.existing_space(session).await else {
            return Vec::new();
        };
        let changes = slot.workspace.lock().await.changes_since(since_version);
        changes
    }
}

#[async_trait]
impl fabric::include::agora::AgoraService for AgoraRegistry {
    async fn view(&self, req: AgoraViewRequest) -> Result<AgoraView> {
        let session = req.space.0.clone();
        let snapshot = self.snapshot(&session).await?;
        let version = self.version(&session).await?;
        Ok(AgoraView {
            space: req.space,
            version,
            snapshot,
        })
    }

    async fn propose(&self, proposal: AgoraProposal) -> Result<uuid::Uuid> {
        let session = proposal.space.0.clone();
        let slot = self.space(&session).await;
        let id = proposal.id;
        slot.workspace.lock().await.propose_full(proposal)?;
        self.proposal_index.lock().await.insert(id, session);
        Ok(id)
    }

    async fn commit(&self, id: uuid::Uuid, permit: WorkspaceCommitPermit) -> Result<CommitReceipt> {
        anyhow::ensure!(permit.proposal_id == id, "commit permit proposal mismatch");
        let session = self
            .proposal_index
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("proposal {id} not found"))?;
        let commit = self
            .commit_transaction(&session, id, Some(&permit))
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(CommitReceipt { commit })
    }

    async fn reject(&self, id: uuid::Uuid, reason: fabric::RejectReason) -> Result<()> {
        let session = self
            .proposal_index
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("proposal {id} not found"))?;
        <Self as AgoraOps>::reject(self, &session, id, reason)
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn changes_since(
        &self,
        space: fabric::AgoraSpaceId,
        version: u64,
    ) -> Result<Vec<AgoraCommit>> {
        Ok(<Self as AgoraOps>::changes_since(self, &space.0, version).await)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_author() -> fabric::ProcessId {
        fabric::ProcessId(uuid::Uuid::from_u128(2))
    }

    #[tokio::test]
    async fn publish_then_recall() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        reg.publish("s1", "k", json!("v")).await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), Some(json!("v")));
    }

    #[tokio::test]
    async fn recall_missing_session_is_none() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        assert_eq!(reg.recall("nope", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn update_merges_patch() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        reg.publish("s1", "a", json!(1)).await.unwrap();
        reg.update("s1", json!({"b": 2})).await.unwrap();
        assert_eq!(reg.recall("s1", "b").await.unwrap(), Some(json!(2)));
    }

    #[tokio::test]
    async fn snapshot_and_clear() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        reg.publish("s1", "k", json!(1)).await.unwrap();
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["blackboard"]["k"], json!(1));
        reg.clear("s1").await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn trace_appends_and_reflects_in_snapshot() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        reg.publish("s1", "k", json!(1)).await.unwrap();
        let before = reg.snapshot("s1").await.unwrap();
        assert_eq!(before["trace_len"], json!(0));
        reg.trace("s1", "tool_result", json!({"call_id": "c1", "ok": true}))
            .await
            .unwrap();
        let after = reg.snapshot("s1").await.unwrap();
        assert_eq!(after["trace_len"], json!(1));
    }

    #[tokio::test]
    async fn record_evidence_survives_snapshot_as_typed() {
        use fabric::Evidence;
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let ev = Evidence::from_tool_result("c1", "bash", "exit 0", false);
        reg.record_evidence("s1", &ev).await.unwrap();

        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["trace_len"], json!(1));
        // Consumer half: the persisted snapshot carries the typed Evidence,
        // recoverable as the same primitive the producer wrote.
        let entry = &snap["trace"][0];
        assert_eq!(entry["kind"], json!("evidence"));
        let back: Evidence = serde_json::from_value(entry["content"].clone()).unwrap();
        assert_eq!(back.id, "c1");
        assert_eq!(back.source, "bash");
        assert_eq!(back.weight, 1.0);
    }

    #[tokio::test]
    async fn propose_commit_changes_since_roundtrip() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let op = AgoraOperation::PublishFact {
            key: "x".into(),
            value: json!(42),
        };
        let prop = reg.propose("s1", 0, op, test_author()).await.unwrap();
        assert_eq!(prop.base_version, 0);

        let commit = reg.commit("s1", prop.id).await.unwrap();
        assert_eq!(commit.id, prop.id);

        let changes = reg.changes_since("s1", 0).await;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, prop.id);
    }

    #[tokio::test]
    async fn propose_conflict_returns_error() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let op1 = AgoraOperation::PublishFact {
            key: "a".into(),
            value: json!(1),
        };
        let p1 = reg.propose("s1", 0, op1, test_author()).await.unwrap();
        reg.commit("s1", p1.id).await.unwrap();

        let op2 = AgoraOperation::PublishFact {
            key: "b".into(),
            value: json!(2),
        };
        let err = reg.propose("s1", 0, op2, test_author()).await.unwrap_err();
        assert!(err.contains("version conflict"));
    }

    #[tokio::test]
    async fn changes_since_missing_session_returns_empty() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let changes = reg.changes_since("nope", 0).await;
        assert!(changes.is_empty());
    }

    // -----------------------------------------------------------------------
    // Phase 3B tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn propose_then_commit_bumps_version() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let op = AgoraOperation::PublishFact {
            key: "z".into(),
            value: json!(99),
        };
        let prop = reg.propose("s1", 0, op, test_author()).await.unwrap();
        assert_eq!(prop.base_version, 0);

        let commit = reg.commit("s1", prop.id).await.unwrap();
        assert_eq!(commit.id, prop.id);

        // Version is reflected in the snapshot (version starts at 0, commit
        // bumps to 1).
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["version"], json!(1));

        // changes_since(0) returns exactly 1 commit.
        let changes = reg.changes_since("s1", 0).await;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, prop.id);
    }

    #[tokio::test]
    async fn propose_wrong_base_version_is_conflict() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));

        // Commit first to bump workspace version to 1.
        let p1 = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "first".into(),
                    value: json!(1),
                },
                test_author(),
            )
            .await
            .unwrap();
        reg.commit("s1", p1.id).await.unwrap();

        // Propose with stale base_version 0 — must return Conflict error.
        let err = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "second".into(),
                    value: json!(2),
                },
                test_author(),
            )
            .await
            .unwrap_err();
        assert!(
            err.contains("version conflict"),
            "expected version conflict, got: {err}"
        );
    }

    #[tokio::test]
    async fn changes_since_returns_only_newer_commits() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));

        // Commit 3 ops: version goes 0→1→2→3.
        for (i, key) in ["a", "b", "c"].iter().enumerate() {
            let base = i as u64;
            let prop = reg
                .propose(
                    "s1",
                    base,
                    AgoraOperation::PublishFact {
                        key: key.to_string(),
                        value: json!(i),
                    },
                    test_author(),
                )
                .await
                .unwrap();
            reg.commit("s1", prop.id).await.unwrap();
        }

        // changes_since(1) returns last 2 commits (versions at index 1 and 2).
        let changes = reg.changes_since("s1", 1).await;
        assert_eq!(changes.len(), 2, "expected 2 commits since version 1");

        // Verify the commit operation keys to confirm they're the later two.
        let keys: Vec<&str> = changes
            .iter()
            .map(|c| match &c.operation {
                AgoraOperation::PublishFact { key, .. } => key.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(keys, vec!["b", "c"]);

        // Edge: changes_since(3) returns empty.
        assert!(reg.changes_since("s1", 3).await.is_empty());
    }

    #[tokio::test]
    async fn publish_still_works_alongside_new_api() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));

        // Old publish/recall API still works.
        reg.publish("s1", "old_key", json!("old_val"))
            .await
            .unwrap();
        assert_eq!(
            reg.recall("s1", "old_key").await.unwrap(),
            Some(json!("old_val"))
        );

        // New propose/commit API works on the same session.
        let prop = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "new_key".into(),
                    value: json!("new_val"),
                },
                test_author(),
            )
            .await
            .unwrap();
        reg.commit("s1", prop.id).await.unwrap();

        // Both values are independently visible.
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["blackboard"]["old_key"], json!("old_val"));
        // new_key was logged as a commit but the blackboard was populated
        // directly via publish, not via commit application — verify commit
        // count grew.
        assert_eq!(snap["commit_count"], json!(1));
        assert_eq!(snap["version"], json!(1));
    }

    #[tokio::test]
    async fn claim_then_release() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let oid = "obj-1";

        // Propose + commit ClaimSharedObject.
        let claim_prop = reg
            .propose(
                "s1",
                0,
                AgoraOperation::ClaimSharedObject { oid: oid.into() },
                test_author(),
            )
            .await
            .unwrap();
        let claim_commit = reg.commit("s1", claim_prop.id).await.unwrap();
        assert_eq!(claim_commit.id, claim_prop.id);

        // Propose + commit ReleaseSharedObject.
        let release_prop = reg
            .propose(
                "s1",
                1,
                AgoraOperation::ReleaseSharedObject { oid: oid.into() },
                test_author(),
            )
            .await
            .unwrap();
        let release_commit = reg.commit("s1", release_prop.id).await.unwrap();
        assert_eq!(release_commit.id, release_prop.id);

        // Both commits appear in the history.
        let changes = reg.changes_since("s1", 0).await;
        assert_eq!(changes.len(), 2);

        // Snapshot reflects 2 commits and version 2.
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["version"], json!(2));
        assert_eq!(snap["commit_count"], json!(2));
    }

    // -----------------------------------------------------------------------
    // Reject tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reject_removes_pending_proposal() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .await
            .unwrap();
        reg.reject("s1", prop.id, fabric::RejectReason::Cancelled)
            .await
            .unwrap();

        // Commit of the rejected proposal should fail.
        let err = reg.commit("s1", prop.id).await.unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn reject_unknown_proposal_returns_error() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let err = reg
            .reject("s1", uuid::Uuid::new_v4(), fabric::RejectReason::Timeout)
            .await
            .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn reject_then_propose_new_works() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        // Propose at v0, then reject.
        let prop1 = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "first".into(),
                    value: json!(1),
                },
                test_author(),
            )
            .await
            .unwrap();
        reg.reject("s1", prop1.id, fabric::RejectReason::Superseded)
            .await
            .unwrap();

        // Propose something new at v0 — still valid (version wasn't bumped).
        let prop2 = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "second".into(),
                    value: json!(2),
                },
                test_author(),
            )
            .await
            .unwrap();
        let commit = reg.commit("s1", prop2.id).await.unwrap();
        assert_eq!(commit.id, prop2.id);

        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["version"], json!(1));
    }

    #[tokio::test]
    async fn reject_records_reason_in_trace() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = reg
            .propose(
                "s1",
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .await
            .unwrap();
        reg.reject(
            "s1",
            prop.id,
            fabric::RejectReason::Invalid("malformed key".into()),
        )
        .await
        .unwrap();

        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["trace_len"], json!(1));
        let reason = snap["trace"][0]["content"]["reason"].as_str().unwrap();
        assert!(
            reason.contains("Invalid"),
            "expected Invalid in reason: {reason}"
        );
    }
}

#[cfg(test)]
mod phase3_service_tests {
    use super::*;
    use fabric::include::agora::{AgoraService, AgoraViewRequest, WorkspaceCommitPermit};
    use fabric::{AgoraSpaceId, Evidence};
    use serde_json::json;

    fn test_author() -> ProcessId {
        ProcessId(uuid::Uuid::from_u128(2))
    }

    fn proposal(space: &str, base_version: u64, op: AgoraOperation) -> AgoraProposal {
        AgoraProposal {
            id: uuid::Uuid::new_v4(),
            space: AgoraSpaceId(space.into()),
            author: test_author(),
            base_version,
            operation: op,
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        }
    }

    #[tokio::test]
    async fn mismatched_permit_is_rejected() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let p = proposal(
            "s1",
            0,
            AgoraOperation::PublishFact {
                key: "k".into(),
                value: json!(1),
            },
        );
        let mut permit = WorkspaceCommitPermit::issue_for(&p, i64::MAX).unwrap();
        permit.process = ProcessId(uuid::Uuid::from_u128(99));
        let id = AgoraService::propose(&reg, p).await.unwrap();
        let err = AgoraService::commit(&reg, id, permit).await.unwrap_err();
        assert!(err.to_string().contains("process mismatch"));
    }

    #[tokio::test]
    async fn expired_proposal_cannot_commit() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let mut p = proposal(
            "s1",
            0,
            AgoraOperation::PublishFact {
                key: "k".into(),
                value: json!(1),
            },
        );
        p.expires_at_ms = Some(0);
        let permit = WorkspaceCommitPermit::issue_for(&p, i64::MAX).unwrap();
        let id = AgoraService::propose(&reg, p).await.unwrap();
        let err = AgoraService::commit(&reg, id, permit).await.unwrap_err();
        assert!(err.to_string().contains("expired") || err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn uncommitted_evidence_is_not_in_shared_view() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let evidence = Evidence::from_tool_result("c1", "bash", "ok", false);
        let p = proposal("s1", 0, AgoraOperation::AcceptEvidence { evidence });
        AgoraService::propose(&reg, p).await.unwrap();

        let view = AgoraService::view(
            &reg,
            AgoraViewRequest {
                space: AgoraSpaceId("s1".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(view.snapshot["trace_len"], json!(0));
        assert_eq!(view.version, 0);
    }

    #[tokio::test]
    async fn committed_evidence_enters_shared_view() {
        let reg = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let evidence = Evidence::from_tool_result("c1", "bash", "ok", false);
        let p = proposal("s1", 0, AgoraOperation::AcceptEvidence { evidence });
        let permit = WorkspaceCommitPermit::issue_for(&p, i64::MAX).unwrap();
        let id = AgoraService::propose(&reg, p).await.unwrap();
        AgoraService::commit(&reg, id, permit).await.unwrap();

        let view = AgoraService::view(
            &reg,
            AgoraViewRequest {
                space: AgoraSpaceId("s1".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(view.snapshot["trace_len"], json!(1));
        assert_eq!(view.snapshot["trace"][0]["kind"], json!("evidence"));
    }
}
