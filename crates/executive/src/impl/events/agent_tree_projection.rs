use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::service::event_projection::{EventProjection, ProjectionDescriptor, ProjectionError};
use fabric::{EventPayload, SpineEvent};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTreeState {
    pub agents: BTreeMap<String, AgentTreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTreeNode {
    pub parent_agent_id: Option<String>,
    pub status: String,
    pub last_sequence: u64,
}

pub struct AgentTreeProjection;

impl EventProjection for AgentTreeProjection {
    type State = AgentTreeState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "agent-tree",
            version: 1,
            accepted_schemas: &[
                fabric::SchemaId::EVENT_AGENT_STARTED_V1,
                fabric::SchemaId::EVENT_AGENT_STOPPED_V1,
                fabric::SchemaId::EVENT_AGENT_FAILED_V1,
            ],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        let EventPayload::Inline { value } = &event.payload else {
            return Err(ProjectionError::InvalidDescriptor(
                "Agent lifecycle event must be inline metadata".into(),
            ));
        };
        let agent = value
            .get("agent_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ProjectionError::InvalidDescriptor("missing Agent identity".into()))?;
        let parent = value
            .get("parent_agent_id")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let status = match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_AGENT_STARTED_V1 => "running",
            fabric::SchemaId::EVENT_AGENT_STOPPED_V1 => "stopped",
            fabric::SchemaId::EVENT_AGENT_FAILED_V1 => "failed",
            _ => return Ok(()),
        };
        state.agents.insert(
            agent.into(),
            AgentTreeNode {
                parent_agent_id: parent,
                status: status.into(),
                last_sequence: event.position.sequence.0,
            },
        );
        Ok(())
    }
}
