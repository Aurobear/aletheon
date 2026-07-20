//! Sanitized, read-only conscious-core inspection protocol.

use serde::{Deserialize, Serialize};

use crate::dasein::SelfVersion;
use crate::{
    AgoraSpaceId, BroadcastEpoch, ContentId, ProcessorHealth, ProcessorId, SalienceVector,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateDisposition {
    pub id: ContentId,
    pub source_kind: String,
    pub content_schema: String,
    pub salience: SalienceVector,
    pub winner: bool,
    pub coalition_member: bool,
    pub visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorProcessorAck {
    pub processor: ProcessorId,
    pub health: ProcessorHealth,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub degraded_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsciousCoreSnapshot {
    pub space: AgoraSpaceId,
    pub epoch: BroadcastEpoch,
    pub dispositions: Vec<CandidateDisposition>,
    pub acknowledgements: Vec<InspectorProcessorAck>,
    pub dasein_version: SelfVersion,
    pub indicator_limitations: Vec<String>,
}

impl ConsciousCoreSnapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.space.0.trim().is_empty(), "inspector space is empty");
        anyhow::ensure!(self.epoch.0 > 0, "inspector epoch is zero");
        anyhow::ensure!(
            self.indicator_limitations
                .iter()
                .all(|value| { !value.trim().is_empty() && value.len() <= 2048 }),
            "inspector limitation text is invalid"
        );
        anyhow::ensure!(
            self.acknowledgements.iter().all(|ack| ack
                .degraded_reason
                .as_ref()
                .is_none_or(|value| !value.trim().is_empty() && value.len() <= 1024)),
            "inspector acknowledgement is invalid"
        );
        Ok(())
    }
}
