//! Explicit bounded adapters between conscious broadcasts and domain facades.

mod agent_processor;
mod corpus_processor;
mod memory_processor;
mod metacog_processor;

use std::sync::Arc;
use std::time::Duration;

use fabric::{
    AgoraSpaceId, Clock, ContentId, MonoDeadline, ProcessId, ProcessorAck, ProcessorId,
    SalienceVector, VisibilityScope, WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent,
    WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use uuid::Uuid;

pub use agent_processor::AgentAdapter;
pub use corpus_processor::CorpusProcessor;
pub use memory_processor::MnemosyneProcessor;
pub use metacog_processor::MetacogProcessor;

pub const PROCESSOR_TTL: Duration = Duration::from_secs(60);
pub const WORKSPACE_NAMESPACE: Uuid = Uuid::from_u128(0x4021_c073_f88b_45c9_b913_89b9_42f8_0671);

pub(crate) struct BoundedAdapter {
    pub id: ProcessorId,
    pub source: ProcessId,
    pub clock: Arc<dyn Clock>,
}

impl BoundedAdapter {
    pub fn new(space: &AgoraSpaceId, id: &str, clock: Arc<dyn Clock>) -> Self {
        Self {
            id: ProcessorId(id.into()),
            source: processor_source(space, id),
            clock,
        }
    }

    pub fn candidate(
        &self,
        broadcast: &WorkspaceBroadcast,
        index: usize,
        content: WorkspaceContent,
        salience: SalienceVector,
        visibility: VisibilityScope,
        mut extra_refs: Vec<String>,
    ) -> WorkspaceCandidate {
        let now = self.clock.mono_now();
        let mut source_refs = vec![
            format!("broadcast:{}:{}", broadcast.space.0, broadcast.epoch.0),
            format!("processor:{}", self.id.0),
        ];
        source_refs.append(&mut extra_refs);
        WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(Uuid::new_v5(
                &WORKSPACE_NAMESPACE,
                format!(
                    "processor:{}:{}:{}:{index}",
                    broadcast.space.0, broadcast.epoch.0, self.id.0
                )
                .as_bytes(),
            )),
            space: broadcast.space.clone(),
            source: self.source,
            turn: None,
            content,
            confidence: 0.8,
            salience,
            provenance: WorkspaceProvenance {
                producer: self.source,
                operation: None,
                source_refs,
                observed_at: self.clock.wall_now(),
            },
            visibility,
            dependencies: broadcast.winner_ids.clone(),
            created_at: now,
            expires_at: Some(MonoDeadline::after(now, PROCESSOR_TTL.as_millis() as u64)),
        }
    }
}

pub(crate) fn acknowledgements(broadcast: &WorkspaceBroadcast) -> Vec<ProcessorAck> {
    broadcast
        .winner_ids
        .iter()
        .map(|content_id| ProcessorAck {
            content_id: *content_id,
            accepted: true,
            detail: None,
        })
        .collect()
}

pub(crate) fn processor_source(space: &AgoraSpaceId, processor: &str) -> ProcessId {
    ProcessId(Uuid::new_v5(
        &WORKSPACE_NAMESPACE,
        format!("{}:{processor}", space.0).as_bytes(),
    ))
}

pub(crate) fn salience(urgency: f32, goal: f32, confidence: f32) -> SalienceVector {
    SalienceVector {
        urgency,
        goal_relevance: goal,
        self_relevance: 0.5,
        novelty: 0.5,
        confidence,
        prediction_error: 0.3,
        affect_intensity: 0.2,
        social_relevance: 0.2,
    }
}

pub(crate) fn broadcast_summary(broadcast: &WorkspaceBroadcast) -> String {
    let summary = broadcast
        .selected
        .iter()
        .filter_map(|candidate| serde_json::to_string(&candidate.content).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let summary = if summary.trim().is_empty() {
        format!("workspace epoch {}", broadcast.epoch.0)
    } else {
        summary
    };
    truncate_bytes(&summary, mnemosyne::RecallRequest::MAX_QUERY_BYTES)
}

pub(crate) fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn truncate_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
