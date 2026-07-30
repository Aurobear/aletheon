mod coding_scorer;
mod contract_issuer;
mod evidence_collector;
mod policy;

pub use coding_scorer::{
    coding_v2_rubric, CodingDimensionScorer, CodingV2Scorer, ScoredEvaluationInput,
};
pub use contract_issuer::{DefaultTaskEvaluationContractIssuer, TaskEvaluationContractIssuer};
pub use evidence_collector::{CodingEvidenceCollector, DefaultCodingEvidenceCollector};
pub use policy::EvaluationSettlementPolicy;

use crate::application::turn_diff_tracker::TurnFileDeltaSnapshot;

#[derive(Debug, Clone, Default)]
pub struct TurnEvaluationArtifacts {
    pub workspace: Option<fabric::WorkspacePolicy>,
    pub profile_name: String,
    pub capability_receipts: Vec<fabric::CapabilityTerminalReceipt>,
    pub file_deltas: Vec<TurnFileDeltaSnapshot>,
    pub runtime_faults: Vec<String>,
    pub supplemental_evidence: Vec<fabric::types::metacognition_evidence::EvidenceItem>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvaluationApplicationError {
    #[error("evaluation contract is invalid: {0}")]
    Contract(#[from] fabric::EvaluationContractError),
    #[error("evaluation request is missing authoritative turn identity")]
    MissingTurnIdentity,
    #[error("unsupported evaluation rubric: {0}")]
    UnsupportedRubric(String),
    #[error("evaluation evidence is invalid: {0}")]
    Evidence(String),
    #[error("evaluation scoring failed: {0}")]
    Scoring(String),
}
