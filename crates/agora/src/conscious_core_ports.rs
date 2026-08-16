//! Executive ports for the recurrent conscious workspace coordinator.

use ::contracts::dasein::SelfTransitionReceipt;
use ::contracts::{
    BroadcastEpoch, MonoTime, ProcessorHealth, ProcessorId, SalienceVector, StructuredSelfView,
    WorkspaceBroadcast, WorkspaceCandidate,
};
use async_trait::async_trait;

/// Compatibility exports for conscious-core callers. The Agent projection
/// contract itself is owned by Runtime; only the conscious implementation
/// remains in Executive.
pub use runtime::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, CandidateSubmissionReceipt,
};

#[async_trait]
pub trait ConsciousCandidatePort: Send + Sync {
    async fn submit_candidate(
        &self,
        submission: CandidateSubmission,
    ) -> anyhow::Result<CandidateSubmissionReceipt>;
}

#[derive(Debug, Clone)]
pub struct DaseinIntegration {
    pub transition: SelfTransitionReceipt,
    pub self_view: StructuredSelfView,
}

#[async_trait]
pub trait DaseinWorkspacePort: Send + Sync {
    async fn modulate_salience(
        &self,
        candidate: &WorkspaceCandidate,
    ) -> anyhow::Result<SalienceVector>;

    async fn integrate_broadcast(
        &self,
        broadcast: &WorkspaceBroadcast,
    ) -> anyhow::Result<DaseinIntegration>;

    async fn self_view(&self) -> anyhow::Result<StructuredSelfView>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorCycleStatus {
    pub processor: ProcessorId,
    pub health: ProcessorHealth,
    pub source_epoch: BroadcastEpoch,
    pub admitted_candidates: Vec<::contracts::ContentId>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConsciousCycleReceipt {
    pub operation_id: ::contracts::OperationId,
    pub depth: u16,
    pub opened_at: MonoTime,
    pub broadcast: Option<WorkspaceBroadcast>,
    pub dasein_transition: Option<SelfTransitionReceipt>,
    pub processors: Vec<ProcessorCycleStatus>,
}
