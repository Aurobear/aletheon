//! Typed client bindings derived from the versioned Fabric protocol model.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AgentSnapshot, ApprovalSnapshot, ItemRecord, SessionId};

pub const CLIENT_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventCursor {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

impl EventCursor {
    pub const fn origin() -> Self {
        Self {
            sequence: 0,
            event_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotRequest {
    #[schemars(with = "String")]
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventSubscription {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub after: EventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientRequest {
    Snapshot(SnapshotRequest),
    Subscribe(EventSubscription),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiSnapshot {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub cursor: EventCursor,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub items: Vec<ItemRecord>,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub approvals: Vec<ApprovalSnapshot>,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub agents: Vec<AgentSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemPhase {
    Started,
    Streaming,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemEvent {
    pub cursor: EventCursor,
    pub item_id: String,
    pub phase: ItemPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalEvent {
    pub cursor: EventCursor,
    #[schemars(with = "serde_json::Value")]
    pub approval: ApprovalSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEvent {
    pub cursor: EventCursor,
    #[schemars(with = "serde_json::Value")]
    pub agent: AgentSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientEvent {
    Snapshot(UiSnapshot),
    Item(ItemEvent),
    Approval(ApprovalEvent),
    Agent(AgentEvent),
    Reconnected(EventCursor),
    Failed {
        cursor: Option<EventCursor>,
        message: String,
    },
}

/// Wire wrapper. Unknown top-level fields are retained explicitly for forward compatibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClientMessage<T> {
    pub protocol_version: u16,
    pub payload: T,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unsupported client protocol version {actual}; expected {expected}")]
pub struct UnsupportedClientVersion {
    pub actual: u16,
    pub expected: u16,
}

impl<T> ClientMessage<T> {
    pub fn v1(payload: T) -> Self {
        Self {
            protocol_version: CLIENT_PROTOCOL_VERSION,
            payload,
            extensions: BTreeMap::new(),
        }
    }

    pub fn into_v1(self) -> Result<T, UnsupportedClientVersion> {
        if self.protocol_version != CLIENT_PROTOCOL_VERSION {
            return Err(UnsupportedClientVersion {
                actual: self.protocol_version,
                expected: CLIENT_PROTOCOL_VERSION,
            });
        }
        Ok(self.payload)
    }
}

#[derive(JsonSchema)]
pub struct ClientProtocolSchema {
    pub request: ClientMessage<ClientRequest>,
    pub event: ClientMessage<ClientEvent>,
}

pub fn client_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ClientProtocolSchema))
        .expect("client protocol schema serializes")
}
