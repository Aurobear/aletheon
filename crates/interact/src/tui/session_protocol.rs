//! JSON-RPC adapter for Fabric's generated typed client contracts.

pub use fabric::protocol::client::{
    ClientEvent, ClientMessage, ClientRequest, EventCursor, EventSubscription, SnapshotRequest,
    UiSnapshot, CLIENT_PROTOCOL_VERSION,
};

pub fn request_to_json(request: ClientRequest, id: u64) -> serde_json::Value {
    let method = match request {
        ClientRequest::Initialize(_) => "initialize",
        ClientRequest::Initialized => "initialized",
        ClientRequest::Snapshot(_) => "session.snapshot",
        ClientRequest::Subscribe(_) => "session.subscribe",
    };
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": method,
        "params": ClientMessage::v1(request),
    })
}

// Compatibility adapters for the pre-Q02 session methods. Their payload value
// types remain Fabric-owned; new snapshot/subscription clients use ClientRequest.
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
            Self::Resume(value) => ("session.resume", serde_json::to_value(value)),
            Self::Fork(value) => ("session.fork", serde_json::to_value(value)),
            Self::Interrupt(value) => ("session.interrupt", serde_json::to_value(value)),
            Self::Replay(value) => ("session.replay", serde_json::to_value(value)),
        };
        serde_json::json!({"jsonrpc":"2.0", "id":id, "method":method,
            "params":params.expect("typed compatibility params serialize")})
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
        serde_json::json!({"jsonrpc":"2.0", "method":"session.notification",
            "params":serde_json::to_value(&self.0).expect("typed notification serializes")})
    }
}
