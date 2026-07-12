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

// Re-export versioned commit types from fabric (single source of truth for
// the trait contract), so consumers can import them from `agora::workspace`.
pub use fabric::include::agora::{
    AgoraCommit, AgoraOperation, AgoraProposal, RejectReason, VersionConflict,
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
            author: fabric::ProcessId(uuid::Uuid::nil()),
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
    pub fn propose_full(
        &mut self,
        proposal: AgoraProposal,
    ) -> Result<AgoraProposal, VersionConflict> {
        if proposal.base_version != self.version {
            return Err(VersionConflict {
                expected: proposal.base_version,
                actual: self.version,
            });
        }
        self.proposals.insert(proposal.id, proposal.clone());
        Ok(proposal)
    }

    /// Commit a previously-created proposal. Returns the resulting commit,
    /// bumps the workspace version, applies the operation to workspace state,
    /// and appends to the commit log.
    pub fn commit(&mut self, proposal_id: Uuid) -> Option<AgoraCommit> {
        let proposal = self.proposals.remove(&proposal_id)?;
        let now_ms = self.clock.wall_now().0;
        if proposal.is_expired_at(now_ms) {
            self.trace.push(
                "proposal_rejected",
                serde_json::json!({
                    "proposal_id": proposal.id.to_string(),
                    "reason": "Timeout",
                    "operation": format!("{:?}", proposal.operation),
                }),
            );
            return None;
        }
        let operation = proposal.operation.clone();
        let next_version = self.version + 1;
        let commit = AgoraCommit {
            id: proposal.id,
            space: proposal.space,
            author: proposal.author,
            version: next_version,
            operation: proposal.operation,
            evidence: proposal.evidence,
            confidence: proposal.confidence,
            committed_at: now_ms,
        };
        self.apply_operation(&operation);
        self.version = next_version;
        self.commits.push(commit.clone());
        Some(commit)
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
    pub fn apply_commit(&mut self, commit: AgoraCommit) -> bool {
        if self.commits.iter().any(|existing| existing.id == commit.id) {
            return false;
        }
        self.apply_operation(&commit.operation);
        self.version = self.version.max(commit.version);
        self.commits.push(commit);
        true
    }

    /// Apply the semantic effect of an operation to workspace state.
    ///
    /// This is called by [`commit`](Self::commit) so that every committed
    /// operation mutates the workspace, not just the append-only log.
    fn apply_operation(&mut self, op: &AgoraOperation) {
        match op {
            AgoraOperation::PublishFact { key, value } => {
                self.blackboard.set(key, value.clone());
            }
            AgoraOperation::ProposePlan { plan } => {
                // Store the plan as structured trace; full task-graph
                // integration (RFC-014 Phase 3B) will materialize tasks
                // from the plan schema.
                self.trace.push("plan", plan.clone());
                self.blackboard.set("current_plan", plan.clone());
            }
            AgoraOperation::UpdateTask { task_patch } => {
                // Apply status/field updates from the patch to matching
                // task-graph nodes when the patch carries an "id" field.
                self.trace.push("task_update", task_patch.clone());
                if let Some(id) = task_patch.get("id").and_then(|v| v.as_str()) {
                    if let Some(status) = task_patch.get("status").and_then(|v| v.as_str()) {
                        let s = match status {
                            "pending" => crate::task_graph::TaskStatus::Pending,
                            "running" => crate::task_graph::TaskStatus::Running,
                            "done" => crate::task_graph::TaskStatus::Done,
                            "failed" => crate::task_graph::TaskStatus::Failed,
                            _ => return,
                        };
                        self.task_graph.set_status(id, s);
                    }
                }
            }
            AgoraOperation::EmitObservation { obs } => {
                self.trace.push("observation", obs.clone());
            }
            AgoraOperation::AcceptEvidence { evidence } => {
                let content = serde_json::to_value(evidence).unwrap_or(serde_json::Value::Null);
                self.trace.push("evidence", content);
            }
            AgoraOperation::ClaimSharedObject { oid } => {
                // Shared-object claims are tracked as present (the caller's
                // process identity lives in the parent commit context, not the
                // operation payload itself).  Mark the oid as claimed with a
                // placeholder process id.
                self.claims
                    .entry(oid.clone())
                    .or_insert(fabric::ProcessId(uuid::Uuid::nil()));
            }
            AgoraOperation::ReleaseSharedObject { oid } => {
                self.claims.remove(oid);
            }
        }
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
            "trace_len": self.trace.len(),
            // Full trace entries (incl. typed RFC-017 objects like Evidence)
            // so the persisted snapshot carries the reasoning trace, not just
            // its length.
            "trace": self.trace.entries(),
            "version": self.version,
            "commit_count": self.commits.len(),
            "claims_count": self.claims.len(),
            "pending_proposals": self.proposals.len(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_includes_session_and_blackboard() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        ws.blackboard.set("goal", json!("ship it"));
        let snap = ws.snapshot();
        assert_eq!(snap["session_id"], json!("s1"));
        assert_eq!(snap["blackboard"]["goal"], json!("ship it"));
    }

    #[test]
    fn clear_resets_state() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        ws.blackboard.set("k", json!(1));
        ws.clear();
        assert!(ws.blackboard.is_empty());
        assert_eq!(ws.session_id, "s1");
    }

    #[test]
    fn version_starts_at_zero() {
        let ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        assert_eq!(ws.version, 0);
    }

    #[test]
    fn propose_succeeds_when_version_matches() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let op = AgoraOperation::PublishFact {
            key: "x".into(),
            value: json!(42),
        };
        let result = ws.propose(0, op);
        assert!(result.is_ok());
        let proposal = result.unwrap();
        assert_eq!(proposal.base_version, 0);
        assert_eq!(ws.proposals.len(), 1);
    }

    #[test]
    fn propose_fails_on_version_conflict() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        // Bump version by committing something
        let op = AgoraOperation::PublishFact {
            key: "x".into(),
            value: json!(1),
        };
        let prop = ws.propose(0, op).unwrap();
        ws.commit(prop.id);
        assert_eq!(ws.version, 1);

        // Now try to propose with stale base_version
        let op2 = AgoraOperation::PublishFact {
            key: "y".into(),
            value: json!(2),
        };
        let err = ws.propose(0, op2).unwrap_err();
        assert_eq!(err.expected, 0);
        assert_eq!(err.actual, 1);
    }

    #[test]
    fn commit_bumps_version_and_logs() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let op = AgoraOperation::PublishFact {
            key: "k".into(),
            value: json!("v"),
        };
        let prop = ws.propose(0, op).unwrap();
        let commit = ws.commit(prop.id).unwrap();
        assert_eq!(commit.id, prop.id);
        assert_eq!(ws.version, 1);
        assert_eq!(ws.commits.len(), 1);
        assert!(ws.proposals.is_empty());
    }

    #[test]
    fn commit_unknown_proposal_returns_none() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        assert!(ws.commit(Uuid::new_v4()).is_none());
    }

    #[test]
    fn changes_since_returns_commits_after_version() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        // Commit v1
        let p1 = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "a".into(),
                    value: json!(1),
                },
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
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let p = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!(1),
                },
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
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "greeting".into(),
                    value: json!("hello"),
                },
            )
            .unwrap();
        ws.commit(prop.id);
        // The blackboard must now contain the committed fact.
        assert_eq!(ws.blackboard.get("greeting").cloned(), Some(json!("hello")));
    }

    #[test]
    fn commit_emit_observation_appends_to_trace() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::EmitObservation {
                    obs: json!({"temp": 72}),
                },
            )
            .unwrap();
        ws.commit(prop.id);
        assert_eq!(ws.trace.len(), 1);
        let entries = ws.trace.entries();
        assert_eq!(entries[0].kind, "observation");
        assert_eq!(entries[0].content["temp"], json!(72));
    }

    #[test]
    fn commit_claim_release_manages_claims() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let oid = "obj-42".to_string();

        // Claim
        let cp = ws
            .propose(0, AgoraOperation::ClaimSharedObject { oid: oid.clone() })
            .unwrap();
        ws.commit(cp.id);
        assert!(ws.claims.contains_key(&oid));
        assert_eq!(ws.version, 1);

        // Release
        let rp = ws
            .propose(1, AgoraOperation::ReleaseSharedObject { oid: oid.clone() })
            .unwrap();
        ws.commit(rp.id);
        assert!(!ws.claims.contains_key(&oid));
        assert_eq!(ws.version, 2);
    }

    #[test]
    fn commit_propose_plan_traces_and_stores_on_blackboard() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let plan = json!({"steps": ["a", "b", "c"]});
        let prop = ws
            .propose(0, AgoraOperation::ProposePlan { plan: plan.clone() })
            .unwrap();
        ws.commit(prop.id);
        // Plan is stored as a structured trace entry.
        assert_eq!(ws.trace.len(), 1);
        assert_eq!(ws.trace.entries()[0].kind, "plan");
        // Plan is also available on the blackboard for quick access.
        assert_eq!(ws.blackboard.get("current_plan").cloned(), Some(plan));
    }

    #[test]
    fn commit_update_task_sets_status() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        ws.task_graph.add("t1", "do the thing", vec![]);
        let patch = json!({"id": "t1", "status": "done"});
        let prop = ws
            .propose(0, AgoraOperation::UpdateTask { task_patch: patch })
            .unwrap();
        ws.commit(prop.id);
        // Task status must have been updated.
        let node = ws.task_graph.get("t1").unwrap();
        assert_eq!(node.status, crate::task_graph::TaskStatus::Done);
        // Trace records the update.
        assert_eq!(ws.trace.entries()[0].kind, "task_update");
    }

    // -- reject behaviour ---------------------------------------------------

    #[test]
    fn reject_removes_pending_proposal() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
            )
            .unwrap();
        assert_eq!(ws.proposals.len(), 1);

        let result = ws.reject(prop.id, RejectReason::Cancelled);
        assert!(result.is_some());
        assert!(ws.proposals.is_empty());
    }

    #[test]
    fn reject_unknown_proposal_returns_none() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        assert!(ws.reject(Uuid::new_v4(), RejectReason::Timeout).is_none());
    }

    #[test]
    fn reject_records_trace_entry() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
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
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
            )
            .unwrap();
        ws.reject(prop.id, RejectReason::Superseded);
        // Reject should NOT bump the version — it's not a commit.
        assert_eq!(ws.version, 0);
        assert!(ws.commits.is_empty());
    }

    #[test]
    fn reject_prevents_later_commit() {
        let mut ws = Workspace::new("s1", Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let prop = ws
            .propose(
                0,
                AgoraOperation::PublishFact {
                    key: "k".into(),
                    value: json!("v"),
                },
            )
            .unwrap();
        let prop_id = prop.id;
        ws.reject(prop_id, RejectReason::Cancelled);
        // Committing the same id after reject should fail.
        assert!(ws.commit(prop_id).is_none());
    }
}
