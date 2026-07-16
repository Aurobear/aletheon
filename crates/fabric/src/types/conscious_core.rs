//! Contracts for the recurrent Dasein–Agora conscious workspace loop.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::dasein::{SelfTransitionReceipt, SelfVersion, Stimmung};
use crate::{
    AgoraSpaceId, BroadcastEpoch, ContentId, MonoDeadline, ProcessId, WorkspaceBroadcast,
    WorkspaceCandidate,
};

pub const MAX_PROCESSOR_RESPONSE_CANDIDATES: usize = 32;
pub const MAX_PROCESSOR_ACKNOWLEDGEMENTS: usize = 64;
pub const MAX_SELF_VIEW_ITEMS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessorId(pub String);

impl ProcessorId {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.0.trim().is_empty() && self.0.len() <= 256,
            "processor ID is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessorHealth {
    Healthy,
    Degraded,
    Overloaded,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessorAck {
    pub content_id: ContentId,
    pub accepted: bool,
    pub detail: Option<String>,
}

impl ProcessorAck {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.detail
                .as_ref()
                .is_none_or(|detail| !detail.trim().is_empty() && detail.len() <= 1024),
            "processor acknowledgement detail is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessorContext {
    pub space: AgoraSpaceId,
    pub source_epoch: BroadcastEpoch,
    pub dasein_version: SelfVersion,
    pub recipient: ProcessId,
    pub agent_root: ProcessId,
    pub recurrence_depth: u16,
    pub deadline: MonoDeadline,
    pub max_candidates: usize,
}

impl ProcessorContext {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.space.0.trim().is_empty(), "processor space is empty");
        anyhow::ensure!(self.source_epoch.0 > 0, "processor source epoch is zero");
        anyhow::ensure!(
            self.max_candidates > 0 && self.max_candidates <= MAX_PROCESSOR_RESPONSE_CANDIDATES,
            "processor candidate budget is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessorResponse {
    pub processor: ProcessorId,
    pub source_epoch: BroadcastEpoch,
    pub health: ProcessorHealth,
    pub candidates: Vec<WorkspaceCandidate>,
    pub acknowledgements: Vec<ProcessorAck>,
    pub detail: Option<String>,
}

impl ProcessorResponse {
    pub fn validate(&self, context: &ProcessorContext) -> anyhow::Result<()> {
        context.validate()?;
        anyhow::ensure!(
            self.source_epoch == context.source_epoch,
            "processor response epoch does not match its source broadcast"
        );
        anyhow::ensure!(
            self.candidates.len() <= context.max_candidates,
            "processor response candidate count exceeds budget"
        );
        self.validate_persisted(&context.space, context.source_epoch)
    }

    pub fn validate_persisted(
        &self,
        space: &AgoraSpaceId,
        epoch: BroadcastEpoch,
    ) -> anyhow::Result<()> {
        self.processor.validate()?;
        anyhow::ensure!(
            self.source_epoch == epoch,
            "processor response epoch does not match storage key"
        );
        anyhow::ensure!(
            self.candidates.len() <= MAX_PROCESSOR_RESPONSE_CANDIDATES,
            "processor response candidate count exceeds hard bound"
        );
        anyhow::ensure!(
            self.acknowledgements.len() <= MAX_PROCESSOR_ACKNOWLEDGEMENTS,
            "processor acknowledgement count exceeds budget"
        );
        anyhow::ensure!(
            self.detail
                .as_ref()
                .is_none_or(|detail| !detail.trim().is_empty() && detail.len() <= 4096),
            "processor response detail is invalid"
        );
        for candidate in &self.candidates {
            candidate.validate()?;
            anyhow::ensure!(
                candidate.space == *space,
                "processor candidate crosses workspace"
            );
            let source_ref = format!("broadcast:{}:{}", space.0, epoch.0);
            anyhow::ensure!(
                candidate.provenance.source_refs.contains(&source_ref),
                "processor candidate is missing source broadcast provenance"
            );
            anyhow::ensure!(
                candidate
                    .provenance
                    .source_refs
                    .contains(&format!("processor:{}", self.processor.0)),
                "processor candidate is missing processor identity provenance"
            );
        }
        for acknowledgement in &self.acknowledgements {
            acknowledgement.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastIntegrationReceipt {
    pub space: AgoraSpaceId,
    pub epoch: BroadcastEpoch,
    pub broadcast_checksum: String,
    pub operation_id: crate::OperationId,
    pub recurrence_depth: u16,
    pub transition: SelfTransitionReceipt,
}

impl BroadcastIntegrationReceipt {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.space.0.trim().is_empty(),
            "integration space is empty"
        );
        anyhow::ensure!(self.epoch.0 > 0, "integration epoch is zero");
        anyhow::ensure!(
            self.broadcast_checksum.len() == 64
                && self
                    .broadcast_checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "integration broadcast checksum is invalid"
        );
        anyhow::ensure!(
            self.transition.current_version.0 == self.transition.previous_version.0 + 1,
            "integration transition does not advance exactly once"
        );
        Ok(())
    }
}

#[async_trait]
pub trait ConsciousProcessor: Send + Sync {
    fn id(&self) -> ProcessorId;

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredSelfView {
    pub version: SelfVersion,
    pub mood: Stimmung,
    pub concerns: Vec<String>,
    pub projection: Option<String>,
    pub protentions: Vec<String>,
}

impl StructuredSelfView {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.concerns.len() <= MAX_SELF_VIEW_ITEMS
                && self.protentions.len() <= MAX_SELF_VIEW_ITEMS,
            "structured SelfView exceeds item budget"
        );
        for text in self
            .concerns
            .iter()
            .chain(self.protentions.iter())
            .chain(self.projection.iter())
        {
            anyhow::ensure!(
                !text.trim().is_empty() && text.len() <= 8192,
                "structured SelfView text is invalid"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextProjectionReceipt {
    pub space: AgoraSpaceId,
    pub broadcast_epoch: Option<BroadcastEpoch>,
    pub workspace_version: Option<u64>,
    pub dasein_version: SelfVersion,
    pub content_ids: Vec<ContentId>,
}

impl ContextProjectionReceipt {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.space.0.trim().is_empty(), "projection space is empty");
        anyhow::ensure!(
            self.content_ids.len() <= crate::MAX_BROADCAST_WINNERS,
            "projection content count exceeds broadcast bound"
        );
        anyhow::ensure!(
            self.broadcast_epoch.is_some() == self.workspace_version.is_some(),
            "projection broadcast epoch and workspace version must appear together"
        );
        let mut unique = self.content_ids.clone();
        unique.sort();
        unique.dedup();
        anyhow::ensure!(
            unique.len() == self.content_ids.len(),
            "projection content IDs are duplicated"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsciousContextProjection {
    pub latest_broadcast: Option<WorkspaceBroadcast>,
    pub self_view: StructuredSelfView,
    pub receipt: ContextProjectionReceipt,
}

impl ConsciousContextProjection {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.self_view.validate()?;
        self.receipt.validate()?;
        anyhow::ensure!(
            self.self_view.version == self.receipt.dasein_version,
            "SelfView version differs from projection receipt"
        );
        match &self.latest_broadcast {
            Some(broadcast) => {
                broadcast.validate()?;
                anyhow::ensure!(
                    broadcast.space == self.receipt.space
                        && Some(broadcast.epoch) == self.receipt.broadcast_epoch
                        && Some(broadcast.workspace_version) == self.receipt.workspace_version
                        && broadcast.winner_ids == self.receipt.content_ids,
                    "latest broadcast differs from projection receipt"
                );
            }
            None => anyhow::ensure!(
                self.receipt.broadcast_epoch.is_none() && self.receipt.content_ids.is_empty(),
                "empty conscious projection contains broadcast references"
            ),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_rejects_cross_epoch_and_unproven_candidates() {
        let context = ProcessorContext {
            space: AgoraSpaceId("session".into()),
            source_epoch: BroadcastEpoch(3),
            dasein_version: SelfVersion(2),
            recipient: ProcessId::new(),
            agent_root: ProcessId::new(),
            recurrence_depth: 1,
            deadline: MonoDeadline(crate::MonoTime(10)),
            max_candidates: 2,
        };
        let response = ProcessorResponse {
            processor: ProcessorId("dasein".into()),
            source_epoch: BroadcastEpoch(2),
            health: ProcessorHealth::Healthy,
            candidates: vec![],
            acknowledgements: vec![],
            detail: None,
        };
        assert!(response.validate(&context).is_err());
    }

    #[test]
    fn projection_receipt_requires_complete_broadcast_identity() {
        let receipt = ContextProjectionReceipt {
            space: AgoraSpaceId("session".into()),
            broadcast_epoch: Some(BroadcastEpoch(1)),
            workspace_version: None,
            dasein_version: SelfVersion(1),
            content_ids: vec![],
        };
        assert!(receipt.validate().is_err());
    }
}
