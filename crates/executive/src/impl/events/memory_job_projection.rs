use serde::{Deserialize, Serialize};

use crate::service::event_projection::{EventProjection, ProjectionDescriptor, ProjectionError};
use fabric::{EventPayload, EventVisibility, ItemPayload, ItemRecord, SpineEvent};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryJobState {
    pub eligible: Vec<MemoryExtractionJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryExtractionJob {
    pub source_event_id: String,
    pub source_sequence: u64,
    pub session_id: String,
    pub item_id: String,
    pub kind: String,
}

#[derive(Deserialize)]
struct ExplicitMemoryCandidate {
    record_id: String,
    kind: String,
}

pub struct MemoryJobProjection;

impl EventProjection for MemoryJobProjection {
    type State = MemoryJobState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "memory-jobs",
            version: 1,
            accepted_schemas: &[
                fabric::SchemaId::TURN_EVENT_V1,
                fabric::SchemaId::EVENT_MEMORY_CANDIDATE_V1,
            ],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        let EventPayload::Inline { value } = &event.payload else {
            return Ok(());
        };
        if event.schema.0 == fabric::SchemaId::EVENT_MEMORY_CANDIDATE_V1 {
            let candidate: ExplicitMemoryCandidate = serde_json::from_value(value.clone())
                .map_err(anyhow::Error::from)
                .map_err(ProjectionError::Storage)?;
            state.eligible.push(MemoryExtractionJob {
                source_event_id: event.position.event_id.to_string(),
                source_sequence: event.position.sequence.0,
                session_id: event.identity.session_id.clone(),
                item_id: candidate.record_id,
                kind: candidate.kind,
            });
            return Ok(());
        }
        if event.visibility != EventVisibility::ModelVisible {
            return Ok(());
        }
        let item: ItemRecord = serde_json::from_value(value.clone())
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        let kind = match item.payload {
            ItemPayload::AssistantMessage { .. } => "assistant_message",
            ItemPayload::ToolResult { .. } => "tool_result",
            _ => return Ok(()),
        };
        state.eligible.push(MemoryExtractionJob {
            source_event_id: event.position.event_id.to_string(),
            source_sequence: event.position.sequence.0,
            session_id: item.session_id.0,
            item_id: item.id.0.to_string(),
            kind: kind.into(),
        });
        Ok(())
    }
}
