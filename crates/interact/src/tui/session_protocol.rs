//! Typed JSON-RPC artifacts for the session lifecycle protocol.

use fabric::{ItemRecord, SessionId, SessionNotification};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ResumeParams {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForkParams {
    pub session_id: SessionId,
    pub through_sequence: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterruptParams {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayParams {
    pub session_id: SessionId,
    pub after_sequence: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum SessionRpcRequest {
    Resume(ResumeParams),
    Fork(ForkParams),
    Interrupt(InterruptParams),
    Replay(ReplayParams),
}

impl SessionRpcRequest {
    pub fn to_json(&self, id: u64) -> serde_json::Value {
        let (method, params) = match self {
            Self::Resume(params) => ("session.resume", serde_json::to_value(params)),
            Self::Fork(params) => ("session.fork", serde_json::to_value(params)),
            Self::Interrupt(params) => ("session.interrupt", serde_json::to_value(params)),
            Self::Replay(params) => ("session.replay", serde_json::to_value(params)),
        };
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.expect("typed session params serialize"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SessionClientNotification(pub SessionNotification);

impl SessionClientNotification {
    pub fn item_appended(item: ItemRecord) -> Self {
        Self(SessionNotification::ItemAppended {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            item,
        })
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session.notification",
            "params": serde_json::to_value(&self.0).expect("typed notification serializes"),
        })
    }
}
