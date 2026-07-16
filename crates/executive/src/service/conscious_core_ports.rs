//! Executive ports for the recurrent conscious workspace coordinator.

use async_trait::async_trait;
use fabric::dasein::{SelfEventId, SelfTransitionReceipt, SelfVersion};
use fabric::{
    BroadcastEpoch, ConsciousContextProjection, MonoTime, ProcessorHealth, ProcessorId,
    SalienceVector, StructuredSelfView, WorkspaceBroadcast, WorkspaceCandidate,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateCause {
    ExternalObservation {
        event_ref: String,
    },
    ProcessorResponse {
        processor: ProcessorId,
        source_epoch: BroadcastEpoch,
    },
    DaseinTransition {
        event_id: SelfEventId,
        version: SelfVersion,
    },
    GovernedActionOutcome {
        permit_id: String,
        operation_id: fabric::OperationId,
    },
}

impl CandidateCause {
    pub fn required_source_ref(&self) -> String {
        match self {
            Self::ExternalObservation { event_ref } => event_ref.clone(),
            Self::ProcessorResponse {
                processor: _,
                source_epoch,
            } => format!("broadcast_epoch:{}", source_epoch.0),
            Self::DaseinTransition { event_id, version } => {
                format!("dasein:{}:v{}", event_id.0, version.0)
            }
            Self::GovernedActionOutcome {
                permit_id,
                operation_id,
            } => format!("capability:{permit_id}:{}", operation_id.0),
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::ExternalObservation { event_ref } => anyhow::ensure!(
                !event_ref.trim().is_empty() && event_ref.len() <= 32 * 1024,
                "external event reference is invalid"
            ),
            Self::ProcessorResponse { processor, .. } => processor.validate()?,
            Self::DaseinTransition { .. } => {}
            Self::GovernedActionOutcome { permit_id, .. } => anyhow::ensure!(
                !permit_id.trim().is_empty() && permit_id.len() <= 1024,
                "capability permit reference is invalid"
            ),
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CandidateSubmission {
    pub candidate: WorkspaceCandidate,
    pub cause: CandidateCause,
}

impl CandidateSubmission {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.candidate.validate()?;
        self.cause.validate()?;
        let required = self.cause.required_source_ref();
        match &self.cause {
            CandidateCause::ProcessorResponse {
                processor,
                source_epoch,
            } => {
                let full = format!("broadcast:{}:{}", self.candidate.space.0, source_epoch.0);
                anyhow::ensure!(
                    self.candidate.provenance.source_refs.contains(&full),
                    "processor submission lacks source broadcast reference"
                );
                anyhow::ensure!(
                    self.candidate
                        .provenance
                        .source_refs
                        .contains(&format!("processor:{}", processor.0)),
                    "processor submission lacks processor identity reference"
                );
            }
            _ => anyhow::ensure!(
                self.candidate.provenance.source_refs.contains(&required),
                "candidate submission lacks its causal source reference"
            ),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateAdmissionStatus {
    Accepted,
    Duplicate,
    RejectedCapacity,
    RejectedSourceQuota,
    RejectedWrongSpace,
    RejectedInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSubmissionReceipt {
    pub candidate_id: fabric::ContentId,
    pub status: CandidateAdmissionStatus,
    pub detail: Option<String>,
}

#[async_trait]
pub trait ConsciousCandidatePort: Send + Sync {
    async fn submit_candidate(
        &self,
        submission: CandidateSubmission,
    ) -> anyhow::Result<CandidateSubmissionReceipt>;
}

#[async_trait]
pub trait LatestConsciousContextPort: Send + Sync {
    async fn latest_context(
        &self,
        space: &fabric::AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection>;
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
    pub admitted_candidates: Vec<fabric::ContentId>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConsciousCycleReceipt {
    pub operation_id: fabric::OperationId,
    pub depth: u16,
    pub opened_at: MonoTime,
    pub broadcast: Option<WorkspaceBroadcast>,
    pub dasein_transition: Option<SelfTransitionReceipt>,
    pub processors: Vec<ProcessorCycleStatus>,
}
