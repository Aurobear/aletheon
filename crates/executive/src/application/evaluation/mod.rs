mod coding_scorer;
mod contract_issuer;
mod evidence_collector;
mod policy;
mod service;

use async_trait::async_trait;

pub use coding_scorer::{
    coding_v2_rubric, CodingDimensionScorer, CodingV2Scorer, ScoredEvaluationInput,
};
pub use contract_issuer::{DefaultTaskEvaluationContractIssuer, TaskEvaluationContractIssuer};
pub use evidence_collector::{CodingEvidenceCollector, DefaultCodingEvidenceCollector};
pub use policy::EvaluationSettlementPolicy;
pub use service::EvaluationService;

use crate::application::turn_diff_tracker::TurnFileDeltaSnapshot;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TurnEvaluationArtifacts {
    pub workspace: Option<fabric::WorkspacePolicy>,
    pub profile_name: String,
    pub capability_receipts: Vec<fabric::CapabilityTerminalReceipt>,
    pub file_deltas: Vec<TurnFileDeltaSnapshot>,
    pub runtime_faults: Vec<String>,
    pub supplemental_evidence: Vec<fabric::types::metacognition_evidence::EvidenceItem>,
}

#[async_trait]
pub trait EvaluationReceiptStore: Send + Sync {
    async fn append_contract(
        &self,
        contract: &fabric::TaskEvaluationContract,
    ) -> anyhow::Result<()>;
    async fn append_evaluation(
        &self,
        snapshot: &fabric::EvaluationEvidenceSnapshot,
        receipt: &fabric::EvaluationReceipt,
    ) -> anyhow::Result<()>;
    async fn get_receipt(
        &self,
        id: &fabric::EvaluationReceiptId,
    ) -> anyhow::Result<Option<fabric::EvaluationReceipt>>;
    async fn get_snapshot(
        &self,
        id: &fabric::EvaluationSnapshotId,
    ) -> anyhow::Result<Option<fabric::EvaluationEvidenceSnapshot>>;
    async fn get_snapshot_for_receipt(
        &self,
        id: &fabric::EvaluationReceiptId,
    ) -> anyhow::Result<Option<fabric::EvaluationEvidenceSnapshot>>;
    async fn latest_for_subject(
        &self,
        subject: &fabric::EvaluationSubject,
    ) -> anyhow::Result<Option<fabric::EvaluationReceipt>>;
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
