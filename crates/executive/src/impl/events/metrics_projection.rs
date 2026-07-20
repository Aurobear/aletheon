use serde::{Deserialize, Serialize};

use crate::service::event_projection::{EventProjection, ProjectionDescriptor, ProjectionError};
use fabric::{EventPayload, ItemPayload, ItemRecord, SpineEvent};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMetricsState {
    pub event_count: u64,
    pub turn_count: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub queue_pressure_events: u64,
    pub total_latency_ms: u64,
}

pub struct MetricsProjection;

impl EventProjection for MetricsProjection {
    type State = EventMetricsState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "event-metrics",
            version: 1,
            accepted_schemas: super::debug_projection::ALL_EVENT_SCHEMAS,
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        state.event_count += 1;
        if event.schema.0 == fabric::SchemaId::EVENT_TOOL_OBSERVATION_V1 {
            state.tool_calls += 1;
            if inline(event)
                .and_then(|value| value.get("detail"))
                .and_then(|value| value.get("is_error"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                state.tool_errors += 1;
            }
        }
        if event.schema.0 == fabric::SchemaId::TURN_EVENT_V1 {
            let Some(value) = inline(event) else {
                return Ok(());
            };
            if let Ok(item) = serde_json::from_value::<ItemRecord>(value.clone()) {
                match item.payload {
                    ItemPayload::UserMessage { .. } => state.turn_count += 1,
                    ItemPayload::ToolCall { .. } => state.tool_calls += 1,
                    ItemPayload::ToolResult { is_error, .. } if is_error => state.tool_errors += 1,
                    _ => {}
                }
            }
            if let Some(usage) = value.get("usage") {
                state.input_tokens += usage
                    .get("input_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                state.output_tokens += usage
                    .get("output_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
            }
            if value
                .get("queue_pressure")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                state.queue_pressure_events += 1;
            }
            state.total_latency_ms += value
                .get("latency_ms")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
        }
        Ok(())
    }
}

fn inline(event: &SpineEvent) -> Option<&serde_json::Value> {
    match &event.payload {
        EventPayload::Inline { value } => Some(value),
        EventPayload::RawObservationRef { .. } => None,
    }
}
