//! Agora (working-memory) trait contract — the shared cognitive workspace.
//!
//! Like the other subsystem contracts in `include/`, this defines the interface
//! Executive and the cognitive subsystems use to read/write the session-scoped
//! blackboard. The implementation lives in the `agora` crate (`AgoraRegistry`).
//!
//! Session-scoped, in-memory. Persists only via `snapshot()` → Mnemosyne.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::types::evidence::Evidence;
use crate::types::operation::ProcessId;
use crate::types::space::AgoraSpaceId;

// ---------------------------------------------------------------------------
// Versioned commit types (RFC-014 Phase 3B)
// ---------------------------------------------------------------------------

/// An operation that can be proposed and committed to the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgoraOperation {
    PublishFact {
        key: String,
        value: serde_json::Value,
    },
    ProposePlan {
        plan: serde_json::Value,
    },
    UpdateTask {
        task_patch: serde_json::Value,
    },
    EmitObservation {
        obs: serde_json::Value,
    },
    AcceptEvidence {
        evidence: Evidence,
    },
    ClaimSharedObject {
        oid: String,
    },
    ReleaseSharedObject {
        oid: String,
    },
}

/// A proposal to apply an operation at a specific workspace version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgoraProposal {
    pub id: Uuid,
    pub space: AgoraSpaceId,
    pub author: ProcessId,
    pub base_version: u64,
    pub operation: AgoraOperation,
    pub evidence: Vec<String>,
    pub confidence: f32,
    /// Expiration deadline as unix milliseconds. `None` means no TTL.
    pub expires_at_ms: Option<i64>,
}

impl AgoraProposal {
    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        self.expires_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
    }
}

impl AgoraOperation {
    /// Stable digest used to bind an authorization decision to one operation.
    pub fn operation_hash(&self) -> anyhow::Result<String> {
        let encoded = serde_json::to_vec(self)?;
        let digest = Sha256::digest(encoded);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

/// A committed operation, permanently recorded in the workspace history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgoraCommit {
    pub id: Uuid,
    pub space: AgoraSpaceId,
    pub author: ProcessId,
    pub base_version: u64,
    pub version: u64,
    pub operation: AgoraOperation,
    pub evidence: Vec<String>,
    pub confidence: f32,
    pub committed_at: i64,
    pub permit_id: Option<Uuid>,
    pub operation_hash: String,
    pub checksum: String,
}

impl AgoraCommit {
    pub fn from_proposal(
        proposal: &AgoraProposal,
        version: u64,
        committed_at: i64,
        permit_id: Option<Uuid>,
    ) -> anyhow::Result<Self> {
        let operation_hash = proposal.operation.operation_hash()?;
        let checksum = commit_checksum(
            proposal.id,
            &proposal.space,
            proposal.author,
            proposal.base_version,
            version,
            &operation_hash,
            &proposal.evidence,
            proposal.confidence,
            permit_id,
            committed_at,
        )?;
        Ok(Self {
            id: proposal.id,
            space: proposal.space.clone(),
            author: proposal.author,
            base_version: proposal.base_version,
            version,
            operation: proposal.operation.clone(),
            evidence: proposal.evidence.clone(),
            confidence: proposal.confidence,
            committed_at,
            permit_id,
            operation_hash,
            checksum,
        })
    }

    pub fn validate_integrity(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == self.base_version + 1,
            "workspace commit version is not contiguous"
        );
        anyhow::ensure!(
            self.operation_hash == self.operation.operation_hash()?,
            "workspace commit operation hash mismatch"
        );
        let checksum = commit_checksum(
            self.id,
            &self.space,
            self.author,
            self.base_version,
            self.version,
            &self.operation_hash,
            &self.evidence,
            self.confidence,
            self.permit_id,
            self.committed_at,
        )?;
        anyhow::ensure!(
            self.checksum == checksum,
            "workspace commit checksum mismatch"
        );
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn commit_checksum(
    id: Uuid,
    space: &AgoraSpaceId,
    author: ProcessId,
    base_version: u64,
    version: u64,
    operation_hash: &str,
    evidence: &[String],
    confidence: f32,
    permit_id: Option<Uuid>,
    committed_at: i64,
) -> anyhow::Result<String> {
    let material = serde_json::json!({
        "id": id,
        "space": space,
        "author": author,
        "base_version": base_version,
        "version": version,
        "operation_hash": operation_hash,
        "evidence": evidence,
        "confidence": confidence,
        "permit_id": permit_id,
        "committed_at": committed_at,
    });
    let digest = Sha256::digest(serde_json::to_vec(&material)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Returned when a proposal's base_version does not match the workspace
/// version (optimistic-concurrency conflict).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConflict {
    pub expected: u64,
    pub actual: u64,
}

/// Reason for rejecting a proposal. Carried alongside the proposal id in
/// [`AgoraOps::reject`] so consumers can distinguish between timed-out,
/// invalid, and superseded proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RejectReason {
    /// The proposal timed out before it could be committed.
    Timeout,
    /// The proposal was invalid (e.g. failed validation, malformed operation).
    Invalid(String),
    /// The proposal was superseded by a newer proposal.
    Superseded,
    /// The proposal was explicitly cancelled by the proposer.
    Cancelled,
    /// Catch-all for other rejection reasons.
    Other(String),
}

// ---------------------------------------------------------------------------
// Transactional service API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgoraViewRequest {
    pub space: AgoraSpaceId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgoraView {
    pub space: AgoraSpaceId,
    pub version: u64,
    pub snapshot: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommitPermit {
    pub permit_id: Uuid,
    pub space: AgoraSpaceId,
    pub proposal_id: Uuid,
    pub process: ProcessId,
    pub operation_hash: String,
    pub expected_version: u64,
    pub expires_at_ms: i64,
}

impl WorkspaceCommitPermit {
    pub fn issue_for(proposal: &AgoraProposal, expires_at_ms: i64) -> anyhow::Result<Self> {
        Ok(Self {
            permit_id: Uuid::new_v4(),
            space: proposal.space.clone(),
            proposal_id: proposal.id,
            process: proposal.author,
            operation_hash: proposal.operation.operation_hash()?,
            expected_version: proposal.base_version,
            expires_at_ms,
        })
    }

    pub fn validate_for(&self, proposal: &AgoraProposal, now_ms: i64) -> anyhow::Result<()> {
        anyhow::ensure!(self.space == proposal.space, "commit permit space mismatch");
        anyhow::ensure!(
            self.proposal_id == proposal.id,
            "commit permit proposal mismatch"
        );
        anyhow::ensure!(
            self.process == proposal.author,
            "commit permit process mismatch"
        );
        anyhow::ensure!(
            self.operation_hash == proposal.operation.operation_hash()?,
            "commit permit operation mismatch"
        );
        anyhow::ensure!(
            self.expected_version == proposal.base_version,
            "commit permit version mismatch"
        );
        anyhow::ensure!(now_ms < self.expires_at_ms, "commit permit expired");
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitReceipt {
    pub commit: AgoraCommit,
}

#[async_trait]
pub trait AgoraService: Send + Sync {
    async fn view(&self, req: AgoraViewRequest) -> Result<AgoraView>;
    async fn propose(&self, proposal: AgoraProposal) -> Result<Uuid>;
    async fn commit(&self, id: Uuid, permit: WorkspaceCommitPermit) -> Result<CommitReceipt>;
    async fn reject(&self, id: Uuid, reason: RejectReason) -> Result<()>;
    async fn changes_since(&self, space: AgoraSpaceId, version: u64) -> Result<Vec<AgoraCommit>>;
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Agora (working-memory) operations — the shared cognitive workspace.
#[async_trait]
pub trait AgoraOps: Send + Sync {
    /// Read a value from a session's blackboard.
    async fn recall(&self, session: &str, key: &str) -> Result<Option<serde_json::Value>>;
    /// Snapshot the entire session workspace (for debug / commit).
    async fn snapshot(&self, session: &str) -> Result<serde_json::Value>;
    /// Return the current workspace version (0-based, incremented on commit).
    async fn version(&self, session: &str) -> Result<u64> {
        let snap = self.snapshot(session).await?;
        Ok(snap.get("version").and_then(|v| v.as_u64()).unwrap_or(0))
    }
    /// Clear a session's workspace.
    async fn clear(&self, session: &str) -> Result<()>;
    /// Append an entry onto a session's reasoning trace.
    async fn trace(&self, session: &str, kind: &str, content: serde_json::Value) -> Result<()>;

    // -- Typed vocabulary (RFC-017) layered over the generic trace --------
    //
    // These default methods lower a cognitive primitive onto the untyped
    // trace so producers speak the RFC-017 vocabulary instead of hand-rolled
    // JSON. Reading them back (via `snapshot`) deserializes into the same
    // type. Add more recorders here as real producers for other primitives
    // (Hypothesis, Narrative, …) appear — not before (YAGNI).

    /// Record a typed [`Evidence`] onto the session's reasoning trace
    /// (trace kind `"evidence"`).
    async fn record_evidence(&self, session: &str, evidence: &Evidence) -> Result<()> {
        let content = serde_json::to_value(evidence)?;
        self.trace(session, "evidence", content).await
    }

    // -- Versioned, proposal-based commits (RFC-014 Phase 3B) -------------

    /// Propose an operation at a specific base version by an identified process.
    /// Returns the proposal on success, or a conflict error if the version is stale.
    async fn propose(
        &self,
        session: &str,
        base_version: u64,
        operation: AgoraOperation,
        author: ProcessId,
    ) -> std::result::Result<AgoraProposal, String> {
        let _ = (session, base_version, operation, author);
        Err("AgoraOps::propose not implemented for this backend".into())
    }

    /// Commit a previously-created proposal by id. Returns the resulting
    /// commit.
    async fn commit(
        &self,
        session: &str,
        proposal_id: Uuid,
    ) -> std::result::Result<AgoraCommit, String> {
        let _ = (session, proposal_id);
        Err("AgoraOps::commit not implemented for this backend".into())
    }

    /// Commit using an authorization bound to the exact proposal operation.
    async fn commit_with_permit(
        &self,
        session: &str,
        proposal_id: Uuid,
        permit: WorkspaceCommitPermit,
    ) -> std::result::Result<AgoraCommit, String> {
        let _ = (session, proposal_id, permit);
        Err("AgoraOps::commit_with_permit not implemented for this backend".into())
    }

    /// Reject a pending proposal by id, removing it from the proposal set.
    ///
    /// After rejection the proposal can no longer be committed. The `reason`
    /// is recorded in the workspace trace for auditability.
    ///
    /// Returns `Ok(())` if the proposal was found and rejected, or an error
    /// string if the proposal does not exist (already committed, already
    /// rejected, or never proposed).
    async fn reject(
        &self,
        session: &str,
        proposal_id: Uuid,
        reason: RejectReason,
    ) -> std::result::Result<(), String> {
        let _ = (session, proposal_id, reason);
        Err("AgoraOps::reject not implemented for this backend".into())
    }

    /// Return all commits with version strictly greater than `since_version`.
    async fn changes_since(&self, session: &str, since_version: u64) -> Vec<AgoraCommit> {
        let _ = (session, since_version);
        Vec::new()
    }

    /// Public watch surface: return commits after `cursor` (poll-style).
    ///
    /// Future poll-style commit watcher — retained as public API surface but not yet called.
    #[allow(dead_code)]
    async fn watch(&self, session: &str, cursor: u64) -> Vec<AgoraCommit> {
        self.changes_since(session, cursor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Captures the last `trace()` call so we can assert `record_evidence`
    /// lowers correctly and round-trips the typed object.
    #[derive(Default)]
    struct SpyAgora {
        last: Mutex<Option<(String, String, serde_json::Value)>>,
    }

    #[async_trait]
    impl AgoraOps for SpyAgora {
        async fn recall(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>> {
            Ok(None)
        }
        async fn snapshot(&self, _: &str) -> Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
        async fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn trace(&self, session: &str, kind: &str, content: serde_json::Value) -> Result<()> {
            *self.last.lock().unwrap() = Some((session.into(), kind.into(), content));
            Ok(())
        }
        async fn reject(
            &self,
            _session: &str,
            _proposal_id: Uuid,
            _reason: RejectReason,
        ) -> std::result::Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn record_evidence_lowers_to_trace_and_roundtrips() {
        let spy = SpyAgora::default();
        let ev = Evidence::from_tool_result("c1", "bash", "exit 0", false);
        spy.record_evidence("s1", &ev).await.unwrap();

        let (session, kind, content) = spy.last.lock().unwrap().clone().unwrap();
        assert_eq!(session, "s1");
        assert_eq!(kind, "evidence");

        // Consumer half: the trace content deserializes back into Evidence.
        let back: Evidence = serde_json::from_value(content).unwrap();
        assert_eq!(back.id, "c1");
        assert_eq!(back.source, "bash");
        assert_eq!(back.weight, 1.0);
    }

    fn permit_proposal() -> AgoraProposal {
        AgoraProposal {
            id: Uuid::from_u128(11),
            space: AgoraSpaceId("space-a".into()),
            author: ProcessId(Uuid::from_u128(12)),
            base_version: 7,
            operation: AgoraOperation::PublishFact {
                key: "answer".into(),
                value: serde_json::json!(42),
            },
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        }
    }

    #[test]
    fn workspace_permit_round_trips_and_accepts_exact_transaction() {
        let proposal = permit_proposal();
        let permit = WorkspaceCommitPermit::issue_for(&proposal, 200).unwrap();
        permit.validate_for(&proposal, 199).unwrap();
        let encoded = serde_json::to_vec(&permit).unwrap();
        assert_eq!(
            serde_json::from_slice::<WorkspaceCommitPermit>(&encoded).unwrap(),
            permit
        );
    }

    #[test]
    fn workspace_permit_rejects_each_mismatch_and_expiry() {
        let proposal = permit_proposal();
        let valid = WorkspaceCommitPermit::issue_for(&proposal, 200).unwrap();
        let mut cases = Vec::new();
        let mut changed = valid.clone();
        changed.space = AgoraSpaceId("other".into());
        cases.push(changed);
        let mut changed = valid.clone();
        changed.proposal_id = Uuid::from_u128(99);
        cases.push(changed);
        let mut changed = valid.clone();
        changed.process = ProcessId(Uuid::from_u128(99));
        cases.push(changed);
        let mut changed = valid.clone();
        changed.operation_hash = "wrong".into();
        cases.push(changed);
        let mut changed = valid.clone();
        changed.expected_version += 1;
        cases.push(changed);
        for permit in cases {
            assert!(permit.validate_for(&proposal, 100).is_err());
        }
        assert!(valid.validate_for(&proposal, 200).is_err());
    }
}
