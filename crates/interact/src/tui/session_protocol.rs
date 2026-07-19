//! JSON-RPC adapter for Fabric's generated typed client contracts.

pub use fabric::protocol::client::{
    ClientEvent, ClientMessage, ClientRequest, EventCursor, EventSubscription, SnapshotRequest,
    UiSnapshot, CLIENT_PROTOCOL_VERSION,
};

pub fn request_to_json(request: ClientRequest, id: u64) -> serde_json::Value {
    request
        .to_json_rpc(id)
        .expect("typed session request serializes")
}

// Compatibility adapters for the pre-Q02 session methods. Their payload value
// types remain Fabric-owned; new snapshot/subscription clients use ClientRequest.
pub use fabric::protocol::client::{
    ResumeParams, SessionForkParams as ForkParams, SessionInterruptParams as InterruptParams,
    SessionReplayParams as ReplayParams,
};
use fabric::{ItemRecord, SessionNotification};

#[derive(Debug, Clone)]
pub enum SessionRpcRequest {
    Resume(ResumeParams),
    Fork(ForkParams),
    Interrupt(InterruptParams),
    Replay(ReplayParams),
}
impl SessionRpcRequest {
    pub fn to_json(&self, id: u64) -> serde_json::Value {
        let request = match self {
            Self::Resume(value) => {
                fabric::protocol::client::ClientRpcRequest::SessionResume(value.clone())
            }
            Self::Fork(value) => {
                fabric::protocol::client::ClientRpcRequest::SessionFork(value.clone())
            }
            Self::Interrupt(value) => {
                fabric::protocol::client::ClientRpcRequest::SessionInterrupt(value.clone())
            }
            Self::Replay(value) => {
                fabric::protocol::client::ClientRpcRequest::SessionReplay(value.clone())
            }
        };
        request
            .to_json_rpc(Some(id))
            .expect("typed compatibility request serializes")
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
        fabric::protocol::client::session_notification_to_json(&self.0)
            .expect("typed notification serializes")
    }
}
