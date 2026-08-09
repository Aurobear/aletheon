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
use fabric::cognitive_workflow::{
    AgoraProjectionReceipt, AgoraProjectionRequest, AgoraTaskProjection, ArtifactLifecycle,
    ClarificationId, ClarificationRecord, ClarificationState, CognitiveArtifactEnvelope,
    CognitiveArtifactId, CognitiveArtifactKind, CognitiveInterruptionId,
    CognitiveInterruptionRecord, CognitiveRoleProfile, CognitiveTaskStatus,
};
use fabric::types::operation::ProcessId;

// Re-export versioned commit types from fabric (single source of truth for
// the trait contract), so consumers can import them from `agora::workspace`.
pub use fabric::include::agora::{
    AgoraCommit, AgoraOperation, AgoraProposal, RejectReason, VersionConflict,
    WorkspaceCommitPermit,
};

/// D4 non-authoritative active-workspace port.
pub mod port;

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
    /// Only committed, digest-validated cognitive artifacts are visible here.
    pub cognitive_artifacts: HashMap<CognitiveArtifactId, CognitiveArtifactEnvelope>,
    pub clarifications: HashMap<ClarificationId, ClarificationRecord>,
    pub interruptions: HashMap<CognitiveInterruptionId, CognitiveInterruptionRecord>,
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
            cognitive_artifacts: HashMap::new(),
            clarifications: HashMap::new(),
            interruptions: HashMap::new(),
            clock,
        }
    }

    fn ensure_disjoint_active_write_scope(
        &self,
        candidate: &fabric::cognitive_workflow::CognitiveTaskNode,
        excluding: Option<&fabric::cognitive_workflow::CognitiveTaskNodeId>,
    ) -> anyhow::Result<()> {
        if !candidate.role.can_write_workspace()
            || !matches!(
                candidate.status,
                CognitiveTaskStatus::Pending | CognitiveTaskStatus::Running
            )
        {
            return Ok(());
        }
        for active in self.task_graph.cognitive_nodes() {
            if excluding == Some(&active.id)
                || !active.role.can_write_workspace()
                || !matches!(
                    active.status,
                    CognitiveTaskStatus::Pending | CognitiveTaskStatus::Running
                )
            {
                continue;
            }
            anyhow::ensure!(
                !scopes_overlap(&candidate.workspace_scope, &active.workspace_scope),
                "parallel write scope overlaps active task {}",
                active.id.0
            );
        }
        Ok(())
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
        let reason_str = format!("{reason:?}");
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
            AgoraOperation::UpsertCognitiveTask { task } => {
                anyhow::ensure!(!task.id.0.trim().is_empty(), "cognitive task id is empty");
                anyhow::ensure!(
                    !task.objective.trim().is_empty(),
                    "cognitive task objective is empty"
                );
                anyhow::ensure!(
                    task.dependencies
                        .iter()
                        .all(|dependency| dependency != &task.id),
                    "cognitive task cannot depend on itself"
                );
                let profile = CognitiveRoleProfile::canonical(task.role);
                profile.validate()?;
                anyhow::ensure!(
                    task.role_profile == profile.reference,
                    "task role profile does not match the host role contract"
                );
                task.budget.validate()?;
                anyhow::ensure!(
                    task.budget.fits_within(&profile.budget),
                    "task budget exceeds its role profile"
                );
                if task.role.can_write_workspace() {
                    anyhow::ensure!(
                        !task.workspace_scope.is_empty()
                            && task.workspace_scope.iter().all(|scope| valid_scope(scope)),
                        "write-capable task requires a valid bounded workspace scope"
                    );
                }
                if let Some(parent_id) = &task.parent_id {
                    let parent = self
                        .task_graph
                        .cognitive(parent_id)
                        .ok_or_else(|| anyhow::anyhow!("cognitive task parent does not exist"))?;
                    anyhow::ensure!(
                        task.budget.fits_within(&parent.budget),
                        "child task budget exceeds its parent budget"
                    );
                }
                if let Some(existing) = self.task_graph.cognitive(&task.id) {
                    anyhow::ensure!(
                        existing.owner == Some(author),
                        "only the current task owner may replace it"
                    );
                    anyhow::ensure!(
                        task.owner == existing.owner,
                        "task ownership requires an explicit handoff operation"
                    );
                    anyhow::ensure!(
                        task.role == existing.role
                            && task.role_profile == existing.role_profile
                            && task.budget == existing.budget
                            && task.workspace_scope == existing.workspace_scope,
                        "task role authority requires an explicit handoff operation"
                    );
                    anyhow::ensure!(
                        task.objective == existing.objective
                            && task.dependencies == existing.dependencies
                            && task.acceptance_criteria == existing.acceptance_criteria
                            && task.required_artifact_kinds == existing.required_artifact_kinds,
                        "task contract metadata is immutable after creation"
                    );
                    anyhow::ensure!(
                        task.artifact_refs == existing.artifact_refs,
                        "task updates cannot rewrite committed artifact references"
                    );
                    anyhow::ensure!(
                        task.parent_id == existing.parent_id,
                        "task parent cannot change after creation"
                    );
                    anyhow::ensure!(
                        valid_cognitive_status_transition(existing.status, task.status),
                        "invalid cognitive task status transition"
                    );
                } else {
                    anyhow::ensure!(
                        task.owner == Some(author),
                        "new cognitive tasks require the creating owner"
                    );
                    anyhow::ensure!(
                        task.artifact_refs.is_empty(),
                        "new cognitive tasks cannot claim committed artifacts"
                    );
                    self.ensure_disjoint_active_write_scope(task, None)?;
                }
            }
            AgoraOperation::HandoffCognitiveTask {
                task_node_id,
                expected_owner,
                new_owner,
                new_role,
                role_profile,
                budget,
                workspace_scope,
            } => {
                let task = self
                    .task_graph
                    .cognitive(task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("handoff task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(*expected_owner),
                    "handoff expected owner mismatch"
                );
                anyhow::ensure!(
                    author == *expected_owner,
                    "only the current owner may hand off a task"
                );
                anyhow::ensure!(
                    expected_owner != new_owner,
                    "handoff requires a distinct new owner"
                );
                let profile = CognitiveRoleProfile::canonical(*new_role);
                profile.validate()?;
                anyhow::ensure!(
                    *role_profile == profile.reference,
                    "handoff role profile does not match host policy"
                );
                budget.validate()?;
                anyhow::ensure!(
                    budget.fits_within(&profile.budget) && budget.fits_within(&task.budget),
                    "handoff budget exceeds role or parent allocation"
                );
                if new_role.can_write_workspace() {
                    anyhow::ensure!(
                        !workspace_scope.is_empty()
                            && workspace_scope.iter().all(|scope| valid_scope(scope)),
                        "write handoff requires valid bounded scope"
                    );
                    let mut candidate = task.clone();
                    candidate.owner = Some(*new_owner);
                    candidate.role = *new_role;
                    candidate.workspace_scope = workspace_scope.clone();
                    self.ensure_disjoint_active_write_scope(&candidate, Some(task_node_id))?;
                } else {
                    anyhow::ensure!(
                        workspace_scope.is_empty(),
                        "read-only role cannot own a write scope"
                    );
                }
            }
            AgoraOperation::CommitCognitiveArtifact { artifact } => {
                artifact.validate()?;
                anyhow::ensure!(
                    artifact.space.0 == self.session_id,
                    "artifact space mismatch"
                );
                anyhow::ensure!(
                    artifact.lifecycle == ArtifactLifecycle::Proposed,
                    "only proposed artifacts may be committed"
                );
                let task = self
                    .task_graph
                    .cognitive(&artifact.task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("artifact task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(author),
                    "only the owning stage may commit a child artifact proposal"
                );
                let profile = CognitiveRoleProfile::canonical(task.role);
                anyhow::ensure!(
                    profile.required_output_artifacts.contains(&artifact.kind()),
                    "artifact kind is not an authorized output of the owning role"
                );
                if matches!(
                    artifact.kind(),
                    CognitiveArtifactKind::Review | CognitiveArtifactKind::Validation
                ) {
                    anyhow::ensure!(
                        task.role.can_validate(),
                        "artifact requires independent validation authority"
                    );
                    let authored_change = self.cognitive_artifacts.values().any(|existing| {
                        existing.task_node_id == artifact.task_node_id
                            && existing.kind() == CognitiveArtifactKind::ChangeSet
                            && existing.author == artifact.author
                    });
                    anyhow::ensure!(
                        !authored_change,
                        "a process cannot independently certify its own change"
                    );
                }
                anyhow::ensure!(
                    !self.cognitive_artifacts.contains_key(&artifact.id),
                    "cognitive artifact already committed"
                );
            }
            AgoraOperation::RecordStageDecision {
                task_node_id,
                decision,
            } => {
                let task = self
                    .task_graph
                    .cognitive(task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("decision task node does not exist"))?;
                if let Some(owner) = task.owner {
                    anyhow::ensure!(owner == author, "only the task owner may decide its stage");
                }
                anyhow::ensure!(
                    !decision.reason.trim().is_empty(),
                    "stage decision reason is empty"
                );
            }
            AgoraOperation::BlockForClarification { clarification } => {
                anyhow::ensure!(
                    clarification.requested_by == author,
                    "clarification requester mismatch"
                );
                anyhow::ensure!(
                    clarification.state == ClarificationState::Pending
                        && clarification.response.is_none()
                        && clarification.response_event_id.is_none(),
                    "new clarification must be pending and unanswered"
                );
                anyhow::ensure!(
                    !clarification.question.trim().is_empty(),
                    "clarification question is empty"
                );
                anyhow::ensure!(
                    clarification.checkpoint.workspace_version == self.version,
                    "clarification checkpoint does not match workspace version"
                );
                let task = self
                    .task_graph
                    .cognitive(&clarification.task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("clarification task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(author),
                    "only the task owner may block it"
                );
                anyhow::ensure!(
                    matches!(
                        task.status,
                        CognitiveTaskStatus::Pending | CognitiveTaskStatus::Running
                    ),
                    "only an active task may request clarification"
                );
                anyhow::ensure!(
                    !self.clarifications.values().any(|existing| {
                        existing.task_node_id == clarification.task_node_id
                            && existing.state == ClarificationState::Pending
                    }),
                    "task already has a pending clarification"
                );
            }
            AgoraOperation::ResolveClarification {
                clarification_id,
                response,
                response_event_id,
            } => {
                anyhow::ensure!(
                    !response.trim().is_empty(),
                    "clarification response is empty"
                );
                anyhow::ensure!(
                    !response_event_id.trim().is_empty(),
                    "canonical response event id is empty"
                );
                let clarification = self
                    .clarifications
                    .get(clarification_id)
                    .ok_or_else(|| anyhow::anyhow!("clarification does not exist"))?;
                anyhow::ensure!(
                    clarification.state == ClarificationState::Pending,
                    "clarification is not pending"
                );
                let task = self
                    .task_graph
                    .cognitive(&clarification.task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("clarification task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(author),
                    "only the task owner may apply a canonical clarification response"
                );
                anyhow::ensure!(
                    task.status == CognitiveTaskStatus::Blocked,
                    "clarification task is not blocked"
                );
            }
            AgoraOperation::CheckpointInterruption { interruption } => {
                anyhow::ensure!(interruption.owner == author, "interruption owner mismatch");
                anyhow::ensure!(
                    interruption.resume_event_id.is_none(),
                    "new interruption cannot already be resumed"
                );
                anyhow::ensure!(
                    interruption.checkpoint.workspace_version == self.version,
                    "interruption checkpoint does not match workspace version"
                );
                let task = self
                    .task_graph
                    .cognitive(&interruption.task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("interruption task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(author),
                    "only the task owner may suspend it"
                );
                anyhow::ensure!(
                    matches!(
                        task.status,
                        CognitiveTaskStatus::Pending | CognitiveTaskStatus::Running
                    ),
                    "only an active task may be interrupted"
                );
                anyhow::ensure!(
                    !self.interruptions.values().any(|existing| {
                        existing.task_node_id == interruption.task_node_id
                            && existing.resume_event_id.is_none()
                    }),
                    "task already has an active interruption"
                );
            }
            AgoraOperation::ResumeInterruption {
                interruption_id,
                resume_event_id,
            } => {
                anyhow::ensure!(
                    !resume_event_id.trim().is_empty(),
                    "resume event id is empty"
                );
                let interruption = self
                    .interruptions
                    .get(interruption_id)
                    .ok_or_else(|| anyhow::anyhow!("interruption does not exist"))?;
                anyhow::ensure!(
                    interruption.resume_event_id.is_none(),
                    "interruption is already resumed"
                );
                let task = self
                    .task_graph
                    .cognitive(&interruption.task_node_id)
                    .ok_or_else(|| anyhow::anyhow!("interruption task node does not exist"))?;
                anyhow::ensure!(
                    task.owner == Some(author),
                    "only the task owner may resume it"
                );
                anyhow::ensure!(
                    task.status == CognitiveTaskStatus::Suspended,
                    "interrupted task is not suspended"
                );
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
            AgoraOperation::UpsertCognitiveTask { task } => {
                self.task_graph.upsert_cognitive(task.clone());
            }
            AgoraOperation::HandoffCognitiveTask {
                task_node_id,
                new_owner,
                new_role,
                role_profile,
                budget,
                workspace_scope,
                ..
            } => {
                let task = self
                    .task_graph
                    .cognitive_mut(task_node_id)
                    .expect("handoff task validated");
                task.owner = Some(*new_owner);
                task.role = *new_role;
                task.role_profile = role_profile.clone();
                task.budget = budget.clone();
                task.workspace_scope = workspace_scope.clone();
            }
            AgoraOperation::CommitCognitiveArtifact { artifact } => {
                let mut committed = artifact.clone();
                committed.lifecycle = ArtifactLifecycle::Committed;
                self.task_graph
                    .attach_artifact(&committed.task_node_id, committed.id.clone());
                self.cognitive_artifacts
                    .insert(committed.id.clone(), committed);
            }
            AgoraOperation::RecordStageDecision {
                task_node_id,
                decision,
            } => {
                self.trace.push(
                    "cognitive_stage_decision",
                    serde_json::json!({
                        "task_node_id": task_node_id,
                        "author": author,
                        "decision": decision,
                    }),
                );
            }
            AgoraOperation::BlockForClarification { clarification } => {
                let task = self
                    .task_graph
                    .cognitive_mut(&clarification.task_node_id)
                    .expect("clarification task validated");
                task.status = CognitiveTaskStatus::Blocked;
                self.clarifications
                    .insert(clarification.id.clone(), clarification.clone());
            }
            AgoraOperation::ResolveClarification {
                clarification_id,
                response,
                response_event_id,
            } => {
                let clarification = self
                    .clarifications
                    .get_mut(clarification_id)
                    .expect("clarification existence validated");
                clarification.state = ClarificationState::Answered;
                clarification.response = Some(response.clone());
                clarification.response_event_id = Some(response_event_id.clone());
                let task = self
                    .task_graph
                    .cognitive_mut(&clarification.task_node_id)
                    .expect("clarification task validated");
                task.status = CognitiveTaskStatus::Running;
            }
            AgoraOperation::CheckpointInterruption { interruption } => {
                let task = self
                    .task_graph
                    .cognitive_mut(&interruption.task_node_id)
                    .expect("interruption task validated");
                task.status = CognitiveTaskStatus::Suspended;
                self.interruptions
                    .insert(interruption.id.clone(), interruption.clone());
            }
            AgoraOperation::ResumeInterruption {
                interruption_id,
                resume_event_id,
            } => {
                let interruption = self
                    .interruptions
                    .get_mut(interruption_id)
                    .expect("interruption existence validated");
                interruption.resume_event_id = Some(resume_event_id.clone());
                let task = self
                    .task_graph
                    .cognitive_mut(&interruption.task_node_id)
                    .expect("interruption task validated");
                task.status = CognitiveTaskStatus::Running;
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

    /// Select a bounded, role-specific projection rather than copying another
    /// Agent's transcript. Selection is deterministic and receipt-backed.
    pub fn project_task(
        &self,
        request: AgoraProjectionRequest,
    ) -> anyhow::Result<AgoraTaskProjection> {
        anyhow::ensure!(
            request.space.0 == self.session_id,
            "projection space mismatch"
        );
        let task = self
            .task_graph
            .cognitive(&request.task_node_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("projection task node does not exist"))?;
        let role_profile = CognitiveRoleProfile::canonical(request.role);
        let allowed = if request.include_kinds.is_empty() {
            role_profile.context_projection.accepted_kinds.clone()
        } else {
            anyhow::ensure!(
                request.include_kinds.iter().all(|kind| role_profile
                    .context_projection
                    .accepted_kinds
                    .contains(kind)),
                "projection requested an artifact kind outside the role contract"
            );
            request.include_kinds.clone()
        };
        let mut candidates = task
            .artifact_refs
            .iter()
            .filter_map(|id| self.cognitive_artifacts.get(id))
            .filter(|artifact| allowed.contains(&artifact.kind()))
            .cloned()
            .collect::<Vec<_>>();
        let limit = request
            .max_artifacts
            .min(role_profile.context_projection.max_artifacts)
            .min(64);
        let omit_count = candidates.len().saturating_sub(limit);
        let omitted_artifact_ids = candidates
            .iter()
            .take(omit_count)
            .map(|artifact| artifact.id.clone())
            .collect::<Vec<_>>();
        if omit_count > 0 {
            candidates.drain(..omit_count);
        }
        let included_artifact_ids = candidates
            .iter()
            .map(|artifact| artifact.id.clone())
            .collect();
        let receipt = AgoraProjectionReceipt {
            projection_id: Uuid::new_v4(),
            space: request.space.clone(),
            workspace_version: self.version,
            task_node_id: request.task_node_id,
            role: request.role,
            included_artifact_ids,
            omitted_artifact_ids: omitted_artifact_ids.clone(),
        };
        let clarification = self
            .clarifications
            .values()
            .find(|clarification| {
                clarification.task_node_id == task.id
                    && clarification.state == ClarificationState::Pending
            })
            .cloned();
        let interruption = self
            .interruptions
            .values()
            .find(|interruption| {
                interruption.task_node_id == task.id && interruption.resume_event_id.is_none()
            })
            .cloned();
        Ok(AgoraTaskProjection {
            space: request.space,
            workspace_version: self.version,
            task,
            artifacts: candidates,
            omitted_artifact_ids,
            clarification,
            interruption,
            receipt,
        })
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
            "cognitive_artifact_count": self.cognitive_artifacts.len(),
            "cognitive_artifacts": self.cognitive_artifacts,
            "clarifications": self.clarifications,
            "interruptions": self.interruptions,
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
        self.cognitive_artifacts.clear();
        self.clarifications.clear();
        self.interruptions.clear();
    }
}

fn valid_scope(scope: &str) -> bool {
    use std::path::Component;
    !scope.trim().is_empty()
        && scope.len() <= 4096
        && !std::path::Path::new(scope)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|left| {
        let left = std::path::Path::new(left);
        right.iter().any(|right| {
            let right = std::path::Path::new(right);
            left.starts_with(right) || right.starts_with(left)
        })
    })
}

fn valid_cognitive_status_transition(
    current: fabric::cognitive_workflow::CognitiveTaskStatus,
    next: fabric::cognitive_workflow::CognitiveTaskStatus,
) -> bool {
    use fabric::cognitive_workflow::CognitiveTaskStatus::*;
    current == next
        || matches!(
            (current, next),
            (Pending, Running | Blocked | Suspended | Cancelled)
                | (
                    Running,
                    Blocked | Suspended | Completed | Failed | Cancelled
                )
                | (Blocked | Suspended, Running | Failed | Cancelled)
        )
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

    fn cognitive_task(
        id: &str,
        owner: ProcessId,
        role: fabric::cognitive_workflow::CognitiveRole,
        scope: &[&str],
    ) -> fabric::cognitive_workflow::CognitiveTaskNode {
        use fabric::cognitive_workflow::*;
        let profile = CognitiveRoleProfile::canonical(role);
        CognitiveTaskNode {
            id: CognitiveTaskNodeId(id.into()),
            parent_id: None,
            objective: "bounded work".into(),
            role,
            stage: CognitiveStage::Contract,
            status: CognitiveTaskStatus::Running,
            owner: Some(owner),
            role_profile: profile.reference,
            budget: profile.budget,
            dependencies: Vec::new(),
            acceptance_criteria: vec!["typed result committed".into()],
            workspace_scope: scope.iter().map(|value| (*value).into()).collect(),
            required_artifact_kinds: Vec::new(),
            artifact_refs: Vec::new(),
            unresolved_finding_ids: Vec::new(),
        }
    }

    #[test]
    fn overlapping_active_write_scopes_are_rejected() {
        use fabric::cognitive_workflow::CognitiveRole;
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        let first_owner = ProcessId::new();
        let first = ws
            .propose(
                0,
                AgoraOperation::UpsertCognitiveTask {
                    task: cognitive_task(
                        "first",
                        first_owner,
                        CognitiveRole::Executor,
                        &["crates/a"],
                    ),
                },
                first_owner,
            )
            .unwrap();
        ws.commit(first.id).unwrap();

        let second_owner = ProcessId::new();
        let second = ws
            .propose(
                1,
                AgoraOperation::UpsertCognitiveTask {
                    task: cognitive_task(
                        "second",
                        second_owner,
                        CognitiveRole::Fixer,
                        &["crates/a/src"],
                    ),
                },
                second_owner,
            )
            .unwrap();
        assert!(ws
            .prepare_commit(second.id, None)
            .unwrap_err()
            .to_string()
            .contains("overlaps"));
        assert_eq!(ws.version, 1);
    }

    #[test]
    fn ownership_handoff_binds_role_profile_budget_and_write_scope() {
        use fabric::cognitive_workflow::*;
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        let executor = ProcessId::new();
        let reviewer = ProcessId::new();
        let create = ws
            .propose(
                0,
                AgoraOperation::UpsertCognitiveTask {
                    task: cognitive_task(
                        "change",
                        executor,
                        CognitiveRole::Executor,
                        &["crates/a"],
                    ),
                },
                executor,
            )
            .unwrap();
        ws.commit(create.id).unwrap();
        let profile = CognitiveRoleProfile::canonical(CognitiveRole::Reviewer);
        let handoff = ws
            .propose(
                1,
                AgoraOperation::HandoffCognitiveTask {
                    task_node_id: CognitiveTaskNodeId("change".into()),
                    expected_owner: executor,
                    new_owner: reviewer,
                    new_role: CognitiveRole::Reviewer,
                    role_profile: profile.reference.clone(),
                    budget: profile.budget.clone(),
                    workspace_scope: Vec::new(),
                },
                executor,
            )
            .unwrap();
        ws.commit(handoff.id).unwrap();
        let task = ws
            .task_graph
            .cognitive(&CognitiveTaskNodeId("change".into()))
            .unwrap();
        assert_eq!(task.owner, Some(reviewer));
        assert_eq!(task.role, CognitiveRole::Reviewer);
        assert_eq!(task.role_profile, profile.reference);
        assert!(task.workspace_scope.is_empty());
    }

    #[test]
    fn snapshot_includes_session_and_blackboard() {
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        ws.blackboard.set("goal", json!("ship it"));
        let snap = ws.snapshot();
        assert_eq!(snap["session_id"], json!("s1"));
        assert_eq!(snap["blackboard"]["goal"], json!("ship it"));
    }

    #[test]
    fn clear_resets_state() {
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        ws.blackboard.set("k", json!(1));
        ws.clear();
        assert!(ws.blackboard.is_empty());
        assert_eq!(ws.session_id, "s1");
    }

    #[test]
    fn version_starts_at_zero() {
        let ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        assert_eq!(ws.version, 0);
    }

    #[test]
    fn propose_succeeds_when_version_matches() {
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        assert!(ws.commit(Uuid::new_v4()).is_none());
    }

    #[test]
    fn changes_since_returns_commits_after_version() {
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        assert!(ws.reject(Uuid::new_v4(), RejectReason::Timeout).is_none());
    }

    #[test]
    fn reject_records_trace_entry() {
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut workspace = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut workspace = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let mut ws = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
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
        let clock = Arc::new(kernel::chronos::TestClock::default());
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
        let mut clean = Workspace::new("s1", Arc::new(kernel::chronos::TestClock::default()));
        assert!(clean.apply_commit(tampered).is_err());
        assert!(clean.attention.focus.is_none());
    }
}
