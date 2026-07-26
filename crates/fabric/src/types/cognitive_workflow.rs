//! Stable cross-crate contracts for versioned cognitive work in Agora.

use crate::{AgoraSpaceId, ProcessId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveTaskNodeId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveRole {
    Root,
    Planner,
    Explorer,
    Executor,
    Reviewer,
    Tester,
    Fixer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveStage {
    Contract,
    Planning,
    Investigation,
    Execution,
    Validation,
    Review,
    Decision,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveTaskStatus {
    Pending,
    Running,
    Blocked,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskNode {
    pub id: CognitiveTaskNodeId,
    pub parent_id: Option<CognitiveTaskNodeId>,
    pub objective: String,
    pub role: CognitiveRole,
    pub stage: CognitiveStage,
    pub status: CognitiveTaskStatus,
    pub owner: Option<ProcessId>,
    pub dependencies: Vec<CognitiveTaskNodeId>,
    pub acceptance_criteria: Vec<String>,
    pub workspace_scope: Vec<String>,
    pub required_artifact_kinds: Vec<CognitiveArtifactKind>,
    pub artifact_refs: Vec<CognitiveArtifactId>,
    pub unresolved_finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveArtifactKind {
    TaskContract,
    Plan,
    Investigation,
    ChangeSet,
    Validation,
    Review,
    Evidence,
    Decision,
    AgentResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactLifecycle {
    Proposed,
    Committed,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveArtifactId(pub Uuid);

impl CognitiveArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CognitiveArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveArtifactEnvelope {
    pub id: CognitiveArtifactId,
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub author: ProcessId,
    pub source_versions: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
    pub lifecycle: ArtifactLifecycle,
    pub artifact: CognitiveArtifact,
    pub content_digest: String,
}

impl CognitiveArtifactEnvelope {
    pub fn proposed(
        space: AgoraSpaceId,
        task_node_id: CognitiveTaskNodeId,
        author: ProcessId,
        source_versions: Vec<String>,
        evidence_refs: Vec<String>,
        confidence: f32,
        artifact: CognitiveArtifact,
    ) -> anyhow::Result<Self> {
        let content_digest = artifact.digest()?;
        Ok(Self {
            id: CognitiveArtifactId::new(),
            space,
            task_node_id,
            author,
            source_versions,
            evidence_refs,
            confidence,
            lifecycle: ArtifactLifecycle::Proposed,
            artifact,
            content_digest,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.content_digest == self.artifact.digest()?,
            "cognitive artifact digest mismatch"
        );
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "cognitive artifact confidence is invalid"
        );
        anyhow::ensure!(
            !self.task_node_id.0.trim().is_empty(),
            "cognitive artifact task node is empty"
        );
        Ok(())
    }

    pub fn kind(&self) -> CognitiveArtifactKind {
        self.artifact.kind()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum CognitiveArtifact {
    TaskContract(CognitiveTaskContractArtifact),
    Plan(CognitivePlanArtifact),
    Investigation(InvestigationReport),
    ChangeSet(CognitiveChangeSetReceipt),
    Validation(CognitiveValidationRecord),
    Review(ReviewFindingSet),
    Evidence(CognitiveEvidenceRecord),
    Decision(StageDecision),
    AgentResult(AgentResultReceipt),
}

impl CognitiveArtifact {
    pub fn kind(&self) -> CognitiveArtifactKind {
        match self {
            Self::TaskContract(_) => CognitiveArtifactKind::TaskContract,
            Self::Plan(_) => CognitiveArtifactKind::Plan,
            Self::Investigation(_) => CognitiveArtifactKind::Investigation,
            Self::ChangeSet(_) => CognitiveArtifactKind::ChangeSet,
            Self::Validation(_) => CognitiveArtifactKind::Validation,
            Self::Review(_) => CognitiveArtifactKind::Review,
            Self::Evidence(_) => CognitiveArtifactKind::Evidence,
            Self::Decision(_) => CognitiveArtifactKind::Decision,
            Self::AgentResult(_) => CognitiveArtifactKind::AgentResult,
        }
    }

    pub fn digest(&self) -> anyhow::Result<String> {
        let bytes = serde_json::to_vec(self)?;
        Ok(Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskContractArtifact {
    pub objective: String,
    pub requirement_refs: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub instruction_refs: Vec<String>,
    pub workspace_scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlanArtifact {
    pub steps: Vec<CognitivePlanArtifactStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlanArtifactStep {
    pub id: String,
    pub description: String,
    pub requirement_refs: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvestigationReport {
    pub summary: String,
    pub findings: Vec<GroundedFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundedFinding {
    pub id: String,
    pub claim: String,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveChangeSetReceipt {
    pub transaction_id: String,
    pub workspace_version: String,
    pub changed_paths: Vec<String>,
    pub diff_artifact_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveValidationRecord {
    pub transaction_id: String,
    pub workspace_version: String,
    pub validation_receipt_refs: Vec<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFindingSet {
    pub transaction_id: String,
    pub workspace_version: String,
    pub findings: Vec<ReviewFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub id: String,
    pub severity: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveEvidenceRecord {
    pub claim: String,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageDecisionKind {
    Accept,
    Reject,
    Replan,
    Repair,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDecision {
    pub decision: StageDecisionKind,
    pub reason: String,
    pub finding_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResultReceipt {
    pub role: CognitiveRole,
    pub operation_id: String,
    pub terminal_status: String,
    pub output_ref: Option<String>,
    pub produced_artifact_refs: Vec<CognitiveArtifactId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraProjectionRequest {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub role: CognitiveRole,
    pub max_artifacts: usize,
    pub include_kinds: Vec<CognitiveArtifactKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgoraTaskProjection {
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub task: CognitiveTaskNode,
    pub artifacts: Vec<CognitiveArtifactEnvelope>,
    pub omitted_artifact_ids: Vec<CognitiveArtifactId>,
    pub clarification: Option<ClarificationRecord>,
    pub interruption: Option<CognitiveInterruptionRecord>,
    pub receipt: AgoraProjectionReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraTaskList {
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub tasks: Vec<CognitiveTaskNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraProjectionReceipt {
    pub projection_id: Uuid,
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub task_node_id: CognitiveTaskNodeId,
    pub role: CognitiveRole,
    pub included_artifact_ids: Vec<CognitiveArtifactId>,
    pub omitted_artifact_ids: Vec<CognitiveArtifactId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClarificationId(pub Uuid);

impl ClarificationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ClarificationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClarificationState {
    Pending,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveCheckpoint {
    pub workspace_version: u64,
    pub task_contract_artifact_ref: Option<CognitiveArtifactId>,
    pub outstanding_obligations: Vec<String>,
    pub validation_artifact_refs: Vec<CognitiveArtifactId>,
    pub runtime_receipt_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClarificationRecord {
    pub id: ClarificationId,
    pub task_node_id: CognitiveTaskNodeId,
    pub requested_by: ProcessId,
    pub question: String,
    pub choices: Vec<String>,
    pub checkpoint: CognitiveCheckpoint,
    pub state: ClarificationState,
    pub response: Option<String>,
    /// Identity of the canonical user-input event that resumed the task.
    pub response_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveInterruptionId(pub Uuid);

impl CognitiveInterruptionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CognitiveInterruptionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveInterruptionReason {
    User,
    DaemonShutdown,
    ProviderUnavailable,
    AuthorityRequired,
    ExternalStateChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveInterruptionRecord {
    pub id: CognitiveInterruptionId,
    pub task_node_id: CognitiveTaskNodeId,
    pub owner: ProcessId,
    pub reason: CognitiveInterruptionReason,
    pub checkpoint: CognitiveCheckpoint,
    pub resume_event_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_digest_binds_typed_content() {
        let mut envelope = CognitiveArtifactEnvelope::proposed(
            AgoraSpaceId("root-task".into()),
            CognitiveTaskNodeId("plan".into()),
            ProcessId::new(),
            vec!["requirements:v1".into()],
            Vec::new(),
            0.9,
            CognitiveArtifact::Plan(CognitivePlanArtifact {
                steps: vec![CognitivePlanArtifactStep {
                    id: "read".into(),
                    description: "read authoritative code".into(),
                    requirement_refs: vec!["spec:section 1".into()],
                    dependencies: Vec::new(),
                }],
            }),
        )
        .unwrap();
        envelope.validate().unwrap();
        let CognitiveArtifact::Plan(plan) = &mut envelope.artifact else {
            unreachable!()
        };
        plan.steps[0].description = "unsupported guess".into();
        assert!(envelope.validate().is_err());
    }
}
