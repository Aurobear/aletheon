//! Provider-neutral Application boundary for conscious turn observation.

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ConsciousTurnObservation {
    pub space: contracts::AgoraSpaceId,
    pub owner: contracts::ProcessId,
    pub root: contracts::ProcessId,
    pub operation: contracts::OperationId,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsciousProcessorStatus {
    pub processor: contracts::ProcessorId,
    pub health: contracts::ProcessorHealth,
    pub source_epoch: contracts::BroadcastEpoch,
    pub admitted_candidates: Vec<contracts::ContentId>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConsciousTurnReceipt {
    pub operation_id: contracts::OperationId,
    pub depth: u16,
    pub opened_at: contracts::MonoTime,
    pub broadcast: Option<contracts::WorkspaceBroadcast>,
    pub dasein_transition: Option<contracts::dasein::SelfTransitionReceipt>,
    pub processors: Vec<ConsciousProcessorStatus>,
}

#[async_trait]
pub trait ConsciousObservationPort: Send + Sync {
    async fn observe_turn(
        &self,
        observation: ConsciousTurnObservation,
    ) -> anyhow::Result<ConsciousTurnReceipt>;
}
