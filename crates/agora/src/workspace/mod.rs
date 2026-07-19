//! Workspace — a single session's cognitive workspace, aggregating all
//! working-memory components (RFC-014).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::attention::Attention;
use crate::blackboard::Blackboard;
use crate::task_graph::TaskGraph;
use crate::trace::Trace;
use fabric::types::operation::ProcessId;

// Re-export versioned commit types from fabric (single source of truth for
// the trait contract), so consumers can import them from `agora::workspace`.
pub use fabric::include::agora::{
    AgoraCommit, AgoraOperation, AgoraProposal, RejectReason, VersionConflict,
    WorkspaceCommitPermit,
};

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

/// One session's in-memory cognitive workspace.
#[derive(Clone)]
pub struct Workspace {
    pub session_id: String,
    pub blackboard: Blackboard,
    pub attention: Attention,
    pub task_graph: TaskGraph,
    pub trace: Trace,
    /// Monotonic version counter incremented on every commit.
    pub version: u64,
    /// Ordered history of committed operations.
    pub commits: Vec<AgoraCommit>,
    /// Pending proposals awaiting commit (keyed by proposal id).
    pub proposals: HashMap<Uuid, AgoraProposal>,
    /// Shared-object claims: oid → owning process.
    pub claims: HashMap<String, fabric::ProcessId>,
    clock: Arc<dyn fabric::Clock>,
}

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspace")
            .field("session_id", &self.session_id)
            .field("blackboard", &self.blackboard)
            .field("attention", &self.attention)
            .field("task_graph", &self.task_graph)
            .field("trace", &self.trace)
            .field("version", &self.version)
            .field("commits", &self.commits)
            .field("proposals", &self.proposals)
            .field("claims", &self.claims)
            .field("clock", &"<Clock>")
            .finish()
    }
}

impl Workspace {
    pub fn new(session_id: impl Into<String>, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            session_id: session_id.into(),
            blackboard: Blackboard::new(),
            attention: Attention::new(),
            task_graph: TaskGraph::new(),
            trace: Trace::new(),
            version: 0,
            commits: Vec::new(),
            proposals: HashMap::new(),
            claims: HashMap::new(),
            clock,
        }
    }

    /// Propose an operation to be applied. Succeeds only if `base_version`
    /// equals the current workspace version (optimistic concurrency).
    pub fn propose(
        &mut self,
        base_version: u64,
        operation: AgoraOperation,
        author: ProcessId,
    ) -> Result<AgoraProposal, VersionConflict> {
        if base_version != self.version {
            return Err(VersionConflict {
                expected: base_version,
                actual: self.version,
            });
        }
        let proposal = AgoraProposal {
            id: Uuid::new_v4(),
            space: fabric::AgoraSpaceId(self.session_id.clone()),
            author,
            base_version,
            operation,
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        };
        self.proposals.insert(proposal.id, proposal.clone());
        Ok(proposal)
    }

    /// Insert a fully-specified proposal from the transactional AgoraService API.
    pub fn propose_full(&mut self, proposal: AgoraProposal) -> anyhow::Result<AgoraProposal> {
        if proposal.base_version != self.version {
            anyhow::bail!(
                "version conflict: expected {}, actual {}",
                proposal.base_version,
                self.version
            );
        }
        anyhow::ensure!(
            proposal.space.0 == self.session_id,
            "proposal space mismatch"
        );
        anyhow::ensure!(
            !self.proposals.contains_key(&proposal.id)
                && !self.commits.iter().any(|commit| commit.id == proposal.id),
            "proposal id already exists"
        );
        self.proposals.insert(proposal.id, proposal.clone());
        Ok(proposal)
    }

    pub fn prepare_commit(
        &self,
        proposal_id: Uuid,
        permit: Option<&WorkspaceCommitPermit>,
    ) -> anyhow::Result<AgoraCommit> {
        let proposal = self
            .proposals
            .get(&proposal_id)
            .ok_or_else(|| anyhow::anyhow!("proposal {proposal_id} not found"))?;
        let now_ms = self.clock.wall_now().0;
        anyhow::ensure!(
            !proposal.is_expired_at(now_ms),
            "proposal {proposal_id} expired"
        );
        anyhow::ensure!(
            proposal.space.0 == self.session_id,
            "proposal belongs to a different workspace"
        );
        anyhow::ensure!(
            proposal.base_version == self.version,
            "version conflict: expected {}, actual {}",
            proposal.base_version,
            self.version
        );
        self.validate_operation(&proposal.operation, proposal.author)?;
        if let Some(permit) = permit {
            permit.validate_for(proposal, now_ms)?;
        }
        AgoraCommit::from_proposal(
            proposal,
            self.version + 1,
            now_ms,
            permit.map(|permit| permit.permit_id),
        )
    }

    pub fn apply_prepared_commit(&mut self, commit: AgoraCommit) -> anyhow::Result<()> {
        commit.validate_integrity()?;
        anyhow::ensure!(commit.space.0 == self.session_id, "commit space mismatch");
        anyhow::ensure!(
            commit.base_version == self.version && commit.version == self.version + 1,
            "workspace changed after commit preparation"
        );
        let proposal = self
            .proposals
            .get(&commit.id)
            .ok_or_else(|| anyhow::anyhow!("proposal {} not found", commit.id))?;
        anyhow::ensure!(
            proposal.operation.operation_hash()? == commit.operation_hash
                && proposal.author == commit.author,
            "prepared commit no longer matches proposal"
        );
        self.apply_operation(&commit.operation, commit.author)?;
        self.proposals.remove(&commit.id);
        self.version = commit.version;
        self.commits.push(commit);
        Ok(())
    }

    /// Deprecated in-memory compatibility wrapper. Canonical callers provide a permit.
    #[deprecated(note = "use prepare_commit with WorkspaceCommitPermit and apply_prepared_commit")]
    pub fn commit(&mut self, proposal_id: Uuid) -> Option<AgoraCommit> {
        let prepared = self.prepare_commit(proposal_id, None).ok()?;
        self.apply_prepared_commit(prepared.clone()).ok()?;
        Some(prepared)
    }

    /// Reject a pending proposal by id. Returns `Some(())` if the proposal
    /// was found and removed, or `None` if it does not exist (already
    /// committed, already rejected, or never proposed).
    ///
    /// The rejection reason is recorded in the trace for auditability.
    pub fn reject(&mut self, proposal_id: Uuid, reason: RejectReason) -> Option<()> {
        let proposal = self.proposals.remove(&proposal_id)?;
        let reason_str = format!("{:?}", reason);
        self.trace.push(
            "proposal_rejected",
            serde_json::json!({
                "proposal_id": proposal.id.to_string(),
                "reason": reason_str,
                "operation": format!("{:?}", proposal.operation),
            }),
        );
        Some(())
    }

    /// Replay a persisted commit idempotently.
    pub fn apply_commit(&mut self, commit: AgoraCommit) -> anyhow::Result<bool> {
        if let Some(existing) = self
            .commits
            .iter()
            .find(|existing| existing.id == commit.id)
        {
            anyhow::ensure!(
                serde_json::to_vec(existing)? == serde_json::to_vec(&commit)?,
                "replayed commit id collision"
            );
            return Ok(false);
        }
        commit.validate_integrity()?;
        anyhow::ensure!(
            commit.space.0 == self.session_id,
            "replayed commit space mismatch"
        );
        anyhow::ensure!(
            commit.base_version == self.version && commit.version == self.version + 1,
            "replayed commit sequence is not contiguous"
        );
        self.validate_operation(&commit.operation, commit.author)?;
        self.apply_operation(&commit.operation, commit.author)?;
        self.version = commit.version;
        self.commits.push(commit);
        Ok(true)
    }

    /// Apply the semantic effect of an operation to workspace state.
    ///
    /// This is called by [`commit`](Self::commit) so that every committed
    /// operation mutates the workspace, not just the append-only log.
    fn validate_operation(&self, op: &AgoraOperation, author: ProcessId) -> anyhow::Result<()> {
        match op {
            AgoraOperation::PublishFact { key, .. } => {
                anyhow::ensure!(
                    !key.trim().is_empty() && key.len() <= 256,
                    "invalid fact key"
                );
            }
            AgoraOperation::ProposePlan { plan } => {
                anyhow::ensure!(
                    plan.as_object().is_some_and(|value| !value.is_empty()),
                    "plan must be a non-empty object"
                );
            }
            AgoraOperation::UpdateTask { task_patch } => {
                let id = task_patch
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("task update requires an id"))?;
                let status = task_patch
                    .get("status")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("task update requires a status"))?;
                let desired = parse_task_status(status)?;
                let current = self
                    .task_graph
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("task {id} does not exist"))?;
                anyhow::ensure!(current.status != desired, "task update is a no-op");
                self.task_graph
                    .validate_transition(id, &desired)
                    .map_err(anyhow::Error::new)?;
            }
            AgoraOperation::EmitObservation { obs } => {
                anyhow::ensure!(!obs.is_null(), "observation cannot be null");
            }
            AgoraOperation::AcceptEvidence { evidence } => {
                anyhow::ensure!(
                    !evidence.id.is_empty() && !evidence.source.is_empty(),
                    "evidence provenance is incomplete"
                );
                anyhow::ensure!(
                    (0.0..=1.0).contains(&evidence.weight),
                    "evidence weight is invalid"
                );
            }
            AgoraOperation::ClaimSharedObject { oid } => {
                anyhow::ensure!(!oid.is_empty(), "claim object id is empty");
                anyhow::ensure!(
                    !self.claims.contains_key(oid),
                    "shared object is already claimed"
                );
            }
            AgoraOperation::ReleaseSharedObject { oid } => {
                anyhow::ensure!(
                    self.claims.get(oid) == Some(&author),
                    "shared object is not owned by process"
                );
            }
            AgoraOperation::UpdateAttention {
                focus,
                priorities,
                selection_ref,
            } => {
                anyhow::ensure!(
                    !selection_ref.trim().is_empty() && selection_ref.len() <= 512,
                    "attention selection reference is invalid"
                );
                anyhow::ensure!(priorities.len() <= 32, "attention priorities exceed limit");
                let mut unique = std::collections::HashSet::with_capacity(priorities.len());
                for priority in priorities {
                    anyhow::ensure!(
                        !priority.trim().is_empty() && priority.len() <= 256,
                        "attention priority is invalid"
                    );
                    anyhow::ensure!(
                        unique.insert(priority),
                        "attention priorities contain duplicates"
                    );
                }
                match focus {
                    Some(focus) => anyhow::ensure!(
                        priorities.first() == Some(focus),
                        "attention focus must be the first priority"
                    ),
                    None => anyhow::ensure!(
                        priorities.is_empty(),
                        "attention priorities require a focus"
                    ),
                }
            }
        }
        Ok(())
    }

    fn apply_operation(&mut self, op: &AgoraOperation, author: ProcessId) -> anyhow::Result<()> {
        match op {
            AgoraOperation::PublishFact { key, value } => {
                self.blackboard.set(key, value.clone());
            }
            AgoraOperation::ProposePlan { plan } => {
                self.blackboard.set("current_plan", plan.clone());
            }
            AgoraOperation::UpdateTask { task_patch } => {
                // Apply status/field updates from the patch to matching
                // task-graph nodes when the patch carries an "id" field.
                if let Some(id) = task_patch.get("id").and_then(|v| v.as_str()) {
                    if let Some(status) = task_patch.get("status").and_then(|v| v.as_str()) {
                        let status = parse_task_status(status)?;
                        self.task_graph
                            .transition(id, status)
                            .map_err(anyhow::Error::new)?;
                    }
                }
            }
            AgoraOperation::EmitObservation { .. } => {}
            AgoraOperation::AcceptEvidence { evidence } => {
                self.trace.push(
                    "evidence",
                    serde_json::json!({
                        "id": evidence.id,
                        "source": evidence.source,
                        "weight": evidence.weight,
                        "content_redacted": true,
                    }),
                );
            }
            AgoraOperation::ClaimSharedObject { oid } => {
                // Track the claim with the author's process identity.
                self.claims.insert(oid.clone(), author);
            }
            AgoraOperation::ReleaseSharedObject { oid } => {
                self.claims.remove(oid);
            }
            AgoraOperation::UpdateAttention {
                focus, priorities, ..
            } => {
                self.attention.focus = focus.clone();
                self.attention.priorities = priorities.clone();
            }
        }
        Ok(())
    }

    /// Return all commits with version strictly greater than `since_version`.
    /// The "version" here is the commit's position in the log (1-indexed).
    pub fn changes_since(&self, since_version: u64) -> Vec<AgoraCommit> {
        let start = since_version as usize; // commits[0] is version 1
        if start >= self.commits.len() {
            return Vec::new();
        }
        self.commits[start..].to_vec()
    }

    /// Snapshot the workspace to JSON (for debug / commit to Mnemosyne).
    pub fn snapshot(&self) -> Value {
        json!({
            "session_id": self.session_id,
            "blackboard": self.blackboard.to_json(),
            "attention": {
                "focus": self.attention.focus,
                "priorities": self.attention.priorities,
            },
            "task_count": self.task_graph.len(),
            "task_graph": self.task_graph,
            "trace_len": self.trace.len(),
            // Full trace entries (incl. typed RFC-017 objects like Evidence)
            // so the persisted snapshot carries the reasoning trace, not just
            // its length.
            "trace": self.trace.entries(),
            "version": self.version,
            "commit_count": self.commits.len(),
            "commits": self.commits,
            "claims_count": self.claims.len(),
            "claims": self.claims,
            "pending_proposals": self.proposals.len(),
            "proposals": self.proposals,
        })
    }

    /// Clear all workspace state (keeps the session id).
    pub fn clear(&mut self) {
        self.blackboard.clear();
        self.attention = Attention::new();
        self.task_graph = TaskGraph::new();
        self.trace.clear();
        self.version = 0;
        self.commits.clear();
        self.proposals.clear();
        self.claims.clear();
    }
}

fn parse_task_status(status: &str) -> anyhow::Result<crate::task_graph::TaskStatus> {
    match status {
        "pending" => Ok(crate::task_graph::TaskStatus::Pending),
        "running" => Ok(crate::task_graph::TaskStatus::Running),
        "done" => Ok(crate::task_graph::TaskStatus::Done),
        "failed" => Ok(crate::task_graph::TaskStatus::Failed),
        _ => anyhow::bail!("unknown task status {status}"),
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_author() -> ProcessId {
        ProcessId(uuid::Uuid::from_u128(1))
    }

    #[test]
    fn snapshot_includes_session_and_blackboard() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        ws.blackboard.set("goal", json!("ship it"));
        let snap = ws.snapshot();
        assert_eq!(snap["session_id"], json!("s1"));
        assert_eq!(snap["blackboard"]["goal"], json!("ship it"));
    }

    #[test]
    fn clear_resets_state() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        ws.blackboard.set("k", json!(1));
        ws.clear();
        assert!(ws.blackboard.is_empty());
        assert_eq!(ws.session_id, "s1");
    }

    #[test]
    fn version_starts_at_zero() {
        let ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        assert_eq!(ws.version, 0);
    }

    #[test]
    fn propose_succeeds_when_version_matches() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let op = AgoraOperation::PublishFact {
            key: "x".into(),
            value: json!(42),
        };
        let result = ws.propose(0, op, test_author());
        assert!(result.is_ok());
        let proposal = result.unwrap();
        assert_eq!(proposal.base_version, 0);
        assert_eq!(ws.proposals.len(), 1);
    }

    #[test]
    fn propose_fails_on_version_conflict() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        // Bump version by committing something
        let op = AgoraOperation::PublishFact {
            key: "x".into(),
            value: json!(1),
        };
        let prop = ws.propose(0, op, test_author()).unwrap();
        ws.commit(prop.id);
        assert_eq!(ws.version, 1);

        // Now try to propose with stale base_version
        let op2 = AgoraOperation::PublishFact {
            key: "y".into(),
            value: json!(2),
        };
        let err = ws.propose(0, op2, test_author()).unwrap_err();
        assert_eq!(err.expected, 0);
        assert_eq!(err.actual, 1);
    }

    #[test]
    fn commit_bumps_version_and_logs() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let op = AgoraOperation::PublishFact {
            key: "k".into(),
            value: json!("v"),
        };
        let prop = ws.propose(0, op, test_author()).unwrap();
        let commit = ws.commit(prop.id).unwrap();
        assert_eq!(commit.id, prop.id);
        assert_eq!(ws.version, 1);
        assert_eq!(ws.commits.len(), 1);
        assert!(ws.proposals.is_empty());
    }

    #[test]
    fn commit_unknown_proposal_returns_none() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        assert!(ws.commit(Uuid::new_v4()).is_none());
    }

    #[test]
    fn changes_since_returns_commits_after_version() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        // Commit v1
        let p1 = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "a".into(),
                    value: json!(1),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(p1.id);
        // Commit v2
        let p2 = ws
            .propose(
                1,
                AgoraOperation::PublishFact {
                    key: "b".into(),
                    value: json!(2),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(p2.id);

        let since_v0 = ws.changes_since(0);
        assert_eq!(since_v0.len(), 2);

        let since_v1 = ws.changes_since(1);
        assert_eq!(since_v1.len(), 1);

        let since_v2 = ws.changes_since(2);
        assert_eq!(since_v2.len(), 0);
    }

    #[test]
    fn clear_resets_version_and_commits() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let p = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!(1),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(p.id);
        assert_eq!(ws.version, 1);
        ws.clear();
        assert_eq!(ws.version, 0);
        assert!(ws.commits.is_empty());
        assert!(ws.proposals.is_empty());
        assert!(ws.claims.is_empty());
    }

    // -- apply_operation behaviour (commit mutates workspace state) --------

    #[test]
    fn commit_publish_fact_writes_to_blackboard() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "greeting".into(),
                    value: json!("hello"),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(prop.id);
        // The blackboard must now contain the committed fact.
        assert_eq!(ws.blackboard.get("greeting").cloned(), Some(json!("hello")));
    }

    #[test]
    fn commit_emit_observation_does_not_duplicate_runtime_trace() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::EmitObservation {
                    obs: json!({"temp": 72}),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(prop.id);
        assert!(ws.trace.is_empty());
    }

    #[test]
    fn commit_claim_release_manages_claims() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let oid = "obj-42".to_string();

        // Claim
        let cp = ws
            .propose(
                0,
                AgoraOperation::ClaimSharedObject { oid: oid.clone() },
                test_author(),
            )
            .unwrap();
        ws.commit(cp.id);
        assert!(ws.claims.contains_key(&oid));
        assert_eq!(ws.version, 1);

        // Release
        let rp = ws
            .propose(
                1,
                AgoraOperation::ReleaseSharedObject { oid: oid.clone() },
                test_author(),
            )
            .unwrap();
        ws.commit(rp.id);
        assert!(!ws.claims.contains_key(&oid));
        assert_eq!(ws.version, 2);
    }

    #[test]
    fn commit_propose_plan_stores_on_blackboard_without_runtime_trace() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let plan = json!({"steps": ["a", "b", "c"]});
        let prop = ws
            .propose(
                0,
                AgoraOperation::ProposePlan { plan: plan.clone() },
                test_author(),
            )
            .unwrap();
        ws.commit(prop.id);
        assert!(ws.trace.is_empty());
        // Plan is also available on the blackboard for quick access.
        assert_eq!(ws.blackboard.get("current_plan").cloned(), Some(plan));
    }

    #[test]
    fn commit_update_task_sets_status() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        ws.task_graph.add("t1", "do the thing", vec![]);
        let patch = json!({"id": "t1", "status": "done"});
        let prop = ws
            .propose(
                0,
                AgoraOperation::UpdateTask { task_patch: patch },
                test_author(),
            )
            .unwrap();
        ws.commit(prop.id);
        // Task status must have been updated.
        let node = ws.task_graph.get("t1").unwrap();
        assert_eq!(node.status, crate::task_graph::TaskStatus::Done);
        assert!(ws.trace.is_empty());
    }

    // -- reject behaviour ---------------------------------------------------

    #[test]
    fn reject_removes_pending_proposal() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .unwrap();
        assert_eq!(ws.proposals.len(), 1);

        let result = ws.reject(prop.id, RejectReason::Cancelled);
        assert!(result.is_some());
        assert!(ws.proposals.is_empty());
    }

    #[test]
    fn reject_unknown_proposal_returns_none() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        assert!(ws.reject(Uuid::new_v4(), RejectReason::Timeout).is_none());
    }

    #[test]
    fn reject_records_trace_entry() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .unwrap();
        ws.reject(prop.id, RejectReason::Invalid("bad input".into()));
        assert_eq!(ws.trace.len(), 1);
        let entry = &ws.trace.entries()[0];
        assert_eq!(entry.kind, "proposal_rejected");
        assert!(entry.content["reason"]
            .as_str()
            .unwrap()
            .contains("Invalid"));
    }

    #[test]
    fn reject_does_not_bump_version() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .unwrap();
        ws.reject(prop.id, RejectReason::Superseded);
        // Reject should NOT bump the version — it's not a commit.
        assert_eq!(ws.version, 0);
        assert!(ws.commits.is_empty());
    }

    #[test]
    fn reject_prevents_later_commit() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
                test_author(),
            )
            .unwrap();
        let prop_id = prop.id;
        ws.reject(prop_id, RejectReason::Cancelled);
        // Committing the same id after reject should fail.
        assert!(ws.commit(prop_id).is_none());
    }

    #[test]
    fn transaction_rechecks_base_version_before_apply() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let first = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "first".into(),
                    value: json!(1),
                },
                test_author(),
            )
            .unwrap();
        let stale = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "stale".into(),
                    value: json!(2),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(first.id).unwrap();
        assert!(ws.prepare_commit(stale.id, None).is_err());
        assert!(ws.proposals.contains_key(&stale.id));
        assert!(ws.blackboard.get("stale").is_none());
    }

    #[test]
    fn transaction_rejects_invalid_and_noop_task_updates() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        ws.task_graph.add("task", "work", Vec::new());
        for (id, patch) in [
            (
                Uuid::from_u128(80),
                json!({"id": "missing", "status": "done"}),
            ),
            (
                Uuid::from_u128(81),
                json!({"id": "task", "status": "unknown"}),
            ),
            (
                Uuid::from_u128(82),
                json!({"id": "task", "status": "pending"}),
            ),
        ] {
            ws.propose_full(AgoraProposal {
                id,
                space: fabric::AgoraSpaceId("s1".into()),
                author: test_author(),
                base_version: 0,
                operation: AgoraOperation::UpdateTask { task_patch: patch },
                evidence: Vec::new(),
                confidence: 1.0,
                expires_at_ms: None,
            })
            .unwrap();
            assert!(ws.prepare_commit(id, None).is_err());
            assert!(ws.proposals.contains_key(&id));
        }
        assert_eq!(ws.version, 0);
    }

    #[test]
    fn transaction_rejects_terminal_task_regression_without_commit() {
        let mut workspace = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        workspace.task_graph.add("task", "work", Vec::new());
        workspace
            .task_graph
            .transition("task", crate::task_graph::TaskStatus::Done)
            .unwrap();
        let proposal = workspace
            .propose(
                0,
                AgoraOperation::UpdateTask {
                    task_patch: json!({"id": "task", "status": "pending"}),
                },
                test_author(),
            )
            .unwrap();
        assert!(workspace.prepare_commit(proposal.id, None).is_err());
        assert_eq!(workspace.version, 0);
        assert_eq!(
            workspace.task_graph.get("task").unwrap().status,
            crate::task_graph::TaskStatus::Done
        );
        assert!(workspace.commits.is_empty());
    }

    #[test]
    fn transaction_rejects_running_task_with_unfinished_dependency() {
        let mut workspace = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        workspace.task_graph.add("dependency", "first", Vec::new());
        workspace
            .task_graph
            .add("task", "second", vec!["dependency".into()]);
        let proposal = workspace
            .propose(
                0,
                AgoraOperation::UpdateTask {
                    task_patch: json!({"id": "task", "status": "running"}),
                },
                test_author(),
            )
            .unwrap();
        let error = workspace.prepare_commit(proposal.id, None).unwrap_err();
        assert!(error.to_string().contains("dependencies are done"));
        assert_eq!(workspace.version, 0);
        assert_eq!(
            workspace.task_graph.get("task").unwrap().status,
            crate::task_graph::TaskStatus::Pending
        );
        assert!(workspace.commits.is_empty());
    }

    #[test]
    fn transaction_snapshot_contains_replayable_claim_and_task_state() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        ws.task_graph.add("task", "work", Vec::new());
        let claim = ws
            .propose(
                0,
                AgoraOperation::ClaimSharedObject {
                    oid: "object".into(),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(claim.id).unwrap();
        let snapshot = ws.snapshot();
        assert_eq!(
            snapshot["task_graph"]["nodes"]["task"]["description"],
            "work"
        );
        assert!(snapshot["claims"]["object"].is_string());
        assert_eq!(snapshot["commits"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn attention_commit_validates_and_applies_atomically() {
        let mut ws = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        for priorities in [vec!["winner".into(), "winner".into()], vec!["other".into()]] {
            let proposal = ws
                .propose(
                    0,
                    AgoraOperation::UpdateAttention {
                        focus: Some("winner".into()),
                        priorities,
                        selection_ref: "selection:1".into(),
                    },
                    test_author(),
                )
                .unwrap();
            assert!(ws.prepare_commit(proposal.id, None).is_err());
            assert!(ws.attention.focus.is_none());
        }

        let proposal = ws
            .propose(
                0,
                AgoraOperation::UpdateAttention {
                    focus: Some("winner".into()),
                    priorities: vec!["winner".into(), "runner-up".into()],
                    selection_ref: "selection:2".into(),
                },
                test_author(),
            )
            .unwrap();
        ws.commit(proposal.id).unwrap();
        assert_eq!(ws.attention.focus.as_deref(), Some("winner"));
        assert_eq!(ws.attention.priorities, vec!["winner", "runner-up"]);
    }

    #[test]
    fn attention_commit_replay_is_idempotent_and_tamper_evident() {
        let clock = Arc::new(aletheon_kernel::chronos::TestClock::default());
        let mut source = Workspace::new("s1", clock.clone());
        let proposal = source
            .propose(
                0,
                AgoraOperation::UpdateAttention {
                    focus: Some("winner".into()),
                    priorities: vec!["winner".into()],
                    selection_ref: "selection:3".into(),
                },
                test_author(),
            )
            .unwrap();
        let commit = source.commit(proposal.id).unwrap();

        let mut recovered = Workspace::new("s1", clock);
        assert!(recovered.apply_commit(commit.clone()).unwrap());
        assert!(!recovered.apply_commit(commit.clone()).unwrap());
        assert_eq!(recovered.attention.focus.as_deref(), Some("winner"));

        let mut tampered = commit;
        if let AgoraOperation::UpdateAttention { priorities, .. } = &mut tampered.operation {
            priorities.push("injected".into());
        }
        let mut clean = Workspace::new(
            "s1",
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        assert!(clean.apply_commit(tampered).is_err());
        assert!(clean.attention.focus.is_none());
    }
}
