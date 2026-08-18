//! Agora-owned ports and policy configuration for the recurrent conscious workspace.

use ::contracts::dasein::SelfTransitionReceipt;
use ::contracts::{
    BroadcastEpoch, MonoTime, ProcessorHealth, ProcessorId, SalienceVector, StructuredSelfView,
    WorkspaceBroadcast, WorkspaceCandidate,
};
use async_trait::async_trait;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone)]
pub struct ConsciousCoreConfig {
    pub arbitration_mode: ::contracts::ConsciousArbitrationMode,
    pub max_processors: usize,
    pub max_processor_concurrency: usize,
    pub max_candidates_per_processor: usize,
    pub max_recurrence_depth: u16,
    pub cycle_timeout: Duration,
    pub processor_timeout: Duration,
    pub candidate_ttl: Duration,
}

impl Default for ConsciousCoreConfig {
    fn default() -> Self {
        Self {
            arbitration_mode: ::contracts::ConsciousArbitrationMode::Observe,
            max_processors: 16,
            max_processor_concurrency: 4,
            max_candidates_per_processor: 8,
            max_recurrence_depth: 4,
            cycle_timeout: Duration::from_secs(10),
            processor_timeout: Duration::from_secs(2),
            candidate_ttl: Duration::from_secs(60),
        }
    }
}

impl ConsciousCoreConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=256).contains(&self.max_processors),
            "conscious processor capacity is invalid"
        );
        anyhow::ensure!(
            (1..=self.max_processors).contains(&self.max_processor_concurrency),
            "conscious processor concurrency is invalid"
        );
        anyhow::ensure!(
            (1..=::contracts::MAX_PROCESSOR_RESPONSE_CANDIDATES)
                .contains(&self.max_candidates_per_processor),
            "processor candidate budget is invalid"
        );
        anyhow::ensure!(
            self.max_recurrence_depth > 0,
            "recurrence depth budget is zero"
        );
        anyhow::ensure!(
            !self.cycle_timeout.is_zero()
                && !self.processor_timeout.is_zero()
                && !self.candidate_ttl.is_zero(),
            "conscious timing budget is zero"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct ProcessorRegistration {
    pub processor: Arc<dyn ::contracts::ConsciousProcessor>,
    pub recipient: ::contracts::ProcessId,
    pub agent_root: ::contracts::ProcessId,
    pub schemas: Vec<::contracts::SchemaId>,
    pub capacity: usize,
    pub deadline_ms: u64,
    pub response_visibility: ::contracts::VisibilityScope,
}

/// Candidate admission contracts are owned by Runtime and consumed here.
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
