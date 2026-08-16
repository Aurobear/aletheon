mod coding_scorer;
mod evidence_collector;
mod service;

mod contract_issuer;
pub use application::evaluation_projection::{
    EvaluationProjection, EvaluationProjectionContext, EvaluationProjectionMetrics,
    EvaluationProjectionRecord, EvaluationProjectionReport, EvaluationProjectionSink,
};
pub use coding_scorer::{
    coding_v2_rubric, CodingDimensionScorer, CodingV2Scorer, ScoredEvaluationInput,
};
pub use contract_issuer::{DefaultTaskEvaluationContractIssuer, TaskEvaluationContractIssuer};
pub use evidence_collector::{CodingEvidenceCollector, DefaultCodingEvidenceCollector};
pub use metacog::evaluation::EvaluationReceiptStore;
pub use metacog::evaluation::EvaluationSettlementPolicy;
pub use service::EvaluationService;

use runtime::turn_diff_tracker::TurnFileDeltaSnapshot;

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
