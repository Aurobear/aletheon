use serde::{Deserialize, Serialize};

use crate::application::event_projection::{
    EventProjection, ProjectionDescriptor, ProjectionError,
};
use fabric::{EventPayload, EventVisibility, SpineEvent};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugGraphState {
    pub edges: Vec<DebugEventEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugEventEdge {
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub sequence: u64,
    pub schema: String,
    pub source: String,
    pub target: String,
    pub summary: String,
}

pub struct DebugProjection;

impl EventProjection for DebugProjection {
    type State = DebugGraphState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "debug-graph",
            version: 1,
            accepted_schemas: ALL_EVENT_SCHEMAS,
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        if event.visibility == EventVisibility::Sensitive {
            return Ok(());
        }
        let summary = match &event.payload {
            EventPayload::RawObservationRef {
                media_type,
                size_bytes,
                ..
            } => format!("raw reference: {media_type}, {size_bytes} bytes"),
            EventPayload::Inline { value } => {
                let kind = value
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .or_else(|| value.get("payload").and_then(payload_kind))
                    .unwrap_or("structured event");
                format!(
                    "{kind} ({} bytes, content redacted)",
                    value.to_string().len()
                )
            }
        };
        state.edges.push(DebugEventEdge {
            event_id: event.position.event_id.to_string(),
            parent_event_id: event.position.parent.map(|parent| parent.0.to_string()),
            sequence: event.position.sequence.0,
            schema: event.schema.0.clone(),
            source: event.envelope.source.0.clone(),
            target: event.envelope.target.0.clone(),
            summary,
        });
        Ok(())
    }
}

fn payload_kind(value: &serde_json::Value) -> Option<&str> {
    value.as_object()?.keys().next().map(String::as_str)
}

pub const ALL_EVENT_SCHEMAS: &[&str] = &[
    fabric::SchemaId::TURN_EVENT_V1,
    fabric::SchemaId::EVENT_TOOL_OBSERVATION_V1,
    fabric::SchemaId::EVENT_AGENT_STARTED_V1,
    fabric::SchemaId::EVENT_AGENT_STOPPED_V1,
    fabric::SchemaId::EVENT_AGENT_FAILED_V1,
    fabric::SchemaId::EVENT_MEMORY_STORED_V1,
    fabric::SchemaId::EVENT_SESSION_CREATED_V1,
    fabric::SchemaId::EVENT_SESSION_FORKED_V1,
    fabric::SchemaId::EVENT_MEMORY_CANDIDATE_V1,
    fabric::SchemaId::EVENT_AGORA_BROADCAST_V1,
    fabric::SchemaId::EVENT_RUNTIME_RESTART_V1,
];
