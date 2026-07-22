use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgoraSpaceId, Clock, ConsciousProcessor, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, WorkspaceBroadcast,
};
use mnemosyne::MemoryWorkspaceProjector;

use super::{acknowledgements, broadcast_summary, truncate_bytes, BoundedAdapter, PROCESSOR_TTL};

/// Bounded M04 recall adapter. Recall is untrusted, private, and can only affect
/// the Self after a later global selection.
pub struct MnemosyneProcessor {
    adapter: BoundedAdapter,
    facade: Arc<dyn mnemosyne::MemoryService>,
}

impl MnemosyneProcessor {
    pub fn new(
        space: &AgoraSpaceId,
        clock: Arc<dyn Clock>,
        memory: Arc<dyn mnemosyne::MemoryService>,
    ) -> Self {
        Self {
            adapter: BoundedAdapter::new(space, "mnemosyne", clock),
            facade: memory,
        }
    }
}

#[async_trait]
impl ConsciousProcessor for MnemosyneProcessor {
    fn id(&self) -> ProcessorId {
        self.adapter.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        for selected in &broadcast.selected {
            if let fabric::WorkspaceContent::Observation(observation) = &selected.content {
                let recording_service = self.facade.clone();
                let content = observation.what.clone();
                let source_id = format!("selected-candidate:{}", selected.id.0);
                let observed = fabric::wall_to_datetime(self.adapter.clock.wall_now());
                let record_id = format!("experience:{}:{}", broadcast.epoch.0, selected.id.0);
                tokio::spawn(async move {
                    if let Err(error) = recording_service
                        .record(mnemosyne::ExperienceEvent::Reflection {
                            content,
                            metadata: mnemosyne::MemoryMetadata::local(
                                record_id, source_id, observed,
                            ),
                        })
                        .await
                    {
                        tracing::warn!(%error, "selected experience recording degraded");
                    }
                });
            }
        }
        let query = broadcast_summary(&broadcast);
        let result = self
            .facade
            .recall(mnemosyne::RecallRequest {
                session: broadcast.space.0.clone(),
                query: truncate_bytes(&query, mnemosyne::RecallRequest::MAX_QUERY_BYTES),
                max_items: context.max_candidates.clamp(1, 4),
                max_content_bytes: 16 * 1024,
                current_at: Some(fabric::wall_to_datetime(self.adapter.clock.wall_now())),
                include_historical: false,
                mode: None,
            })
            .await
            .and_then(|recall| {
                let projection = mnemosyne::DefaultMemoryWorkspaceProjector.project(
                    &recall,
                    mnemosyne::MemoryProjectionLimits {
                        max_items: context.max_candidates.clamp(1, 8),
                        ..Default::default()
                    },
                )?;
                let degraded = projection.degraded_sources.clone();
                let mut candidates =
                    projection.to_candidates(&mnemosyne::MemoryCandidateContext {
                        space: broadcast.space.clone(),
                        source: self.adapter.source,
                        source_epoch: broadcast.epoch,
                        dependencies: broadcast.winner_ids.clone(),
                        created_at: self.adapter.clock.mono_now(),
                        ttl_ms: PROCESSOR_TTL.as_millis() as u64,
                    })?;
                for candidate in &mut candidates {
                    candidate
                        .provenance
                        .source_refs
                        .push(format!("processor:{}", self.adapter.id.0));
                }
                Ok((candidates, degraded))
            });
        match result {
            Ok((candidates, degraded)) => ProcessorResponse {
                processor: self.id(),
                source_epoch: context.source_epoch,
                health: if degraded.is_empty() {
                    ProcessorHealth::Healthy
                } else {
                    ProcessorHealth::Degraded
                },
                candidates,
                acknowledgements: acknowledgements(&broadcast),
                detail: (!degraded.is_empty())
                    .then(|| format!("memory sources degraded: {}", degraded.join(","))),
            },
            Err(error) => ProcessorResponse {
                processor: self.id(),
                source_epoch: context.source_epoch,
                health: ProcessorHealth::Degraded,
                candidates: vec![],
                acknowledgements: acknowledgements(&broadcast),
                detail: Some(format!("bounded recall failed: {error}")),
            },
        }
    }
}
