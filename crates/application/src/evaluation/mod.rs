mod evidence_collector;
mod service;

mod contract_issuer;
pub use crate::evaluation_projection::{
    EvaluationProjection, EvaluationProjectionContext, EvaluationProjectionMetrics,
    EvaluationProjectionRecord, EvaluationProjectionReport, EvaluationProjectionSink,
};
pub use contract_issuer::{DefaultTaskEvaluationContractIssuer, TaskEvaluationContractIssuer};
pub use evidence_collector::{
    CodingEvidenceCollector, DefaultCodingEvidenceCollector, EvaluationPathResolver,
};
pub use evidence_collector::{SCOPE_SUMMARY_SOURCE, VERIFICATION_SUMMARY_SOURCE};
pub use service::{
    EvaluationEnginePort, EvaluationOperationPort, EvaluationReceiptStore, EvaluationService,
};

use runtime::turn_diff_tracker::TurnFileDeltaSnapshot;

#[async_trait::async_trait]
pub trait EvaluationAcceptancePort: Send + Sync {
    async fn settle(
        &self,
        session_id: &str,
        receipt: &::contracts::EvaluationReceiptRef,
        owner: ::contracts::ProcessId,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct ScoredEvaluationInput {
    pub dimensions: Vec<::contracts::types::metacognition_evaluation::DimensionScore>,
    pub gates: Vec<::contracts::types::metacognition_evaluation::GateResult>,
}

pub trait CodingDimensionScorer: Send + Sync {
    fn score(
        &self,
        contract: &::contracts::TaskEvaluationContract,
        evidence: &::contracts::EvaluationEvidenceSnapshot,
    ) -> Result<ScoredEvaluationInput, EvaluationApplicationError>;
}

pub struct EvaluationSettlementPolicy;

impl EvaluationSettlementPolicy {
    pub fn settle_stop(
        decision: ::contracts::EvaluationDecision,
        current: ::contracts::TurnStop,
    ) -> ::contracts::TurnStop {
        match decision {
            ::contracts::EvaluationDecision::Rejected
            | ::contracts::EvaluationDecision::Indeterminate => ::contracts::TurnStop::Blocked,
            _ => current,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TurnEvaluationArtifacts {
    /// Canonical thread/session scope used by downstream domain projections.
    #[serde(default)]
    pub session_id: String,
    /// Typed host runtime identity. This is never inferred from model output.
    #[serde(default)]
    pub runtime_id: String,
    /// Typed effective provider/model route selected by the host.
    #[serde(default)]
    pub effective_model_id: String,
    /// Typed display name for the effective model route.
    #[serde(default)]
    pub model_display_name: String,
    pub workspace: Option<::contracts::WorkspacePolicy>,
    pub profile_name: String,
    pub capability_receipts: Vec<::contracts::CapabilityTerminalReceipt>,
    pub file_deltas: Vec<TurnFileDeltaSnapshot>,
    pub runtime_faults: Vec<String>,
    pub supplemental_evidence: Vec<::contracts::types::metacognition_evidence::EvidenceItem>,
    #[serde(default)]
    pub projection_metrics: EvaluationProjectionMetrics,
}

impl TurnEvaluationArtifacts {
    pub fn native(
        session_id: String,
        effective_model_id: String,
        model_display_name: String,
        workspace: ::contracts::WorkspacePolicy,
        profile_name: String,
        mut capability_receipts: Vec<::contracts::CapabilityTerminalReceipt>,
        file_deltas: Vec<TurnFileDeltaSnapshot>,
        runtime_faults: Vec<String>,
        projection_metrics: EvaluationProjectionMetrics,
    ) -> Self {
        capability_receipts.sort_by(|left, right| {
            left.finished_at
                .cmp(&right.finished_at)
                .then_with(|| left.invocation_id.cmp(&right.invocation_id))
        });
        Self {
            session_id,
            runtime_id: "native-turn".into(),
            effective_model_id,
            model_display_name,
            workspace: Some(workspace),
            profile_name,
            capability_receipts,
            file_deltas,
            runtime_faults,
            supplemental_evidence: Vec::new(),
            projection_metrics,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvaluationApplicationError {
    #[error("evaluation contract is invalid: {0}")]
    Contract(#[from] ::contracts::EvaluationContractError),
    #[error("evaluation request is missing authoritative turn identity")]
    MissingTurnIdentity,
    #[error("unsupported evaluation rubric: {0}")]
    UnsupportedRubric(String),
    #[error("evaluation evidence is invalid: {0}")]
    Evidence(String),
    #[error("evaluation scoring failed: {0}")]
    Scoring(String),
}

/// Consumer-owned evaluation receipt port (M8.4 HandlerPorts narrowing).
/// The daemon handler reads evaluation receipts through this trait instead of
/// the concrete `EvaluationService`.
#[async_trait::async_trait]
pub trait EvaluationPort: Send + Sync {
    async fn receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationReceipt>>;
    async fn evidence_snapshot_for_receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationEvidenceSnapshot>>;
}

#[async_trait::async_trait]
impl EvaluationPort for service::EvaluationService {
    async fn receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationReceipt>> {
        self.receipt(id).await
    }
    async fn evidence_snapshot_for_receipt(
        &self,
        id: &::contracts::EvaluationReceiptId,
    ) -> anyhow::Result<Option<::contracts::EvaluationEvidenceSnapshot>> {
        self.evidence_snapshot_for_receipt(id).await
    }
}
