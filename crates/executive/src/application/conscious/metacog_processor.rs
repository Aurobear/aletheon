use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgoraSpaceId, Clock, ConsciousProcessor, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, VisibilityScope, WorkspaceBroadcast, WorkspaceContent,
};

use super::{acknowledgements, salience, BoundedAdapter};

/// Read-only F01 metacognitive facade. It emits proposals; it has no Dasein
/// mutation authority.
pub struct MetacogProcessor {
    adapter: BoundedAdapter,
}

impl MetacogProcessor {
    pub fn new(space: &AgoraSpaceId, clock: Arc<dyn Clock>) -> Self {
        Self {
            adapter: BoundedAdapter::new(space, "metacog", clock),
        }
    }
}

#[async_trait]
impl ConsciousProcessor for MetacogProcessor {
    fn id(&self) -> ProcessorId {
        self.adapter.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        let average_confidence = broadcast
            .selected
            .iter()
            .map(|item| item.confidence)
            .sum::<f32>()
            / broadcast.selected.len().max(1) as f32;
        let conflict = broadcast.selected.iter().any(|left| {
            broadcast.selected.iter().any(|right| {
                left.source != right.source
                    && left.content_fingerprint().ok() == right.content_fingerprint().ok()
            })
        });
        let payload = serde_json::json!({
            "calibration": { "mean_confidence": average_confidence },
            "uncertainty": (1.0 - average_confidence).clamp(0.0, 1.0),
            "conflict": conflict,
            "governed_mutation_proposals": [],
            "authority": "proposal_only"
        });
        let candidate = self.adapter.candidate(
            &broadcast,
            0,
            WorkspaceContent::Extension {
                schema: "v1/metacog/deliberation".into(),
                payload,
            },
            salience(0.4, 0.5, 0.6),
            VisibilityScope::Session,
            vec![format!("dasein-version:{}", context.dasein_version.0)],
        );
        ProcessorResponse {
            processor: self.id(),
            source_epoch: context.source_epoch,
            health: ProcessorHealth::Healthy,
            candidates: vec![candidate],
            acknowledgements: acknowledgements(&broadcast),
            detail: None,
        }
    }
}
