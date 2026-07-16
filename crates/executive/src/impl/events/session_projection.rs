use std::collections::BTreeMap;

use fabric::{EventPayload, EventVisibility, ItemRecord, SessionId, SpineEvent};
use serde::{Deserialize, Serialize};

use crate::service::event_projection::{EventProjection, ProjectionDescriptor, ProjectionError};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionState {
    pub sessions: BTreeMap<String, PublicSessionView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionView {
    pub turns: BTreeMap<String, Vec<u64>>,
    pub items: Vec<ItemRecord>,
}

pub struct SessionProjection;

impl EventProjection for SessionProjection {
    type State = PublicSessionState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "public-session",
            version: 1,
            accepted_schemas: &[fabric::SchemaId::TURN_EVENT_V1],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        if event.visibility == EventVisibility::Sensitive {
            return Ok(());
        }
        let EventPayload::Inline { value } = &event.payload else {
            return Ok(());
        };
        let item: ItemRecord = serde_json::from_value(value.clone())
            .map_err(anyhow::Error::from)
            .map_err(ProjectionError::Storage)?;
        if item.session_id != SessionId(event.identity.session_id.clone())
            || item.sequence != event.position.sequence.0
        {
            return Err(ProjectionError::InvalidDescriptor(
                "Session item identity or order differs from spine".into(),
            ));
        }
        let session = state.sessions.entry(item.session_id.0.clone()).or_default();
        if session
            .items
            .last()
            .is_some_and(|prior| prior.sequence >= item.sequence)
        {
            return Err(ProjectionError::NonMonotonic {
                previous: session.items.last().unwrap().sequence,
                current: item.sequence,
            });
        }
        session
            .turns
            .entry(item.turn_id.0.to_string())
            .or_default()
            .push(item.sequence);
        session.items.push(item);
        Ok(())
    }
}
