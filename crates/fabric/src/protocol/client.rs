//! Typed client bindings derived from the versioned Fabric protocol model.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ui_event::{CollaborationMode, InterruptReason},
    AgentSnapshot, ApprovalSnapshot, ConnectionId, ItemRecord, LocalOsPrincipal, PrincipalId,
    SessionId, WorkspacePolicy,
};

pub const CLIENT_PROTOCOL_VERSION: u16 = 1;
pub const JSON_RPC_VERSION: &str = "2.0";
const SUPPORTED_CLIENT_PROTOCOL_VERSIONS: &[u16] = &[CLIENT_PROTOCOL_VERSION];

/// Typed compatibility requests for the daemon's line-delimited JSON-RPC
/// transport. Method names and parameter field names live in Fabric rather
/// than being re-created by each Interact entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub enum ClientRpcRequest {
    Chat(ChatParams),
    Clear,
    Status,
    Reflect,
    ReflectNow,
    Evolution,
    Genome,
    Sessions,
    Resume(ResumeParams),
    Compact,
    ModelList,
    ModeSwitch(ModeSwitchParams),
    PlanApprove,
    Interrupt(InterruptParams),
    HooksList,
    ApprovalResponse(ApprovalResponseParams),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ChatParams {
    pub message: String,
    pub working_dir: PathBuf,
    pub workspace_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ResumeParams {
    #[schemars(with = "String")]
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ModeSwitchParams {
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InterruptParams {
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransientApprovalDecision {
    Approve,
    ApproveForSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ApprovalResponseParams {
    pub approval_id: String,
    pub decision: TransientApprovalDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: Option<u64>,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

impl ClientRpcRequest {
    pub fn chat(message: impl Into<String>, workspace: &WorkspacePolicy) -> Self {
        Self::Chat(ChatParams {
            message: message.into(),
            working_dir: workspace.cwd().to_path_buf(),
            workspace_roots: workspace.writable_roots().to_vec(),
        })
    }

    pub fn resume(session_id: impl Into<SessionId>) -> Self {
        Self::Resume(ResumeParams {
            session_id: session_id.into(),
        })
    }

    pub fn mode_switch(mode: CollaborationMode) -> Self {
        Self::ModeSwitch(ModeSwitchParams {
            mode: mode.display_name().to_owned(),
        })
    }

    pub fn interrupt(reason: InterruptReason) -> Self {
        let reason = match reason {
            InterruptReason::UserCancelled => "user_cancelled",
            InterruptReason::Timeout => "timeout",
            InterruptReason::BudgetExceeded => "budget_exceeded",
        };
        Self::Interrupt(InterruptParams {
            reason: reason.to_owned(),
        })
    }

    pub fn approval_response(
        approval_id: impl Into<String>,
        decision: TransientApprovalDecision,
    ) -> Self {
        Self::ApprovalResponse(ApprovalResponseParams {
            approval_id: approval_id.into(),
            decision,
        })
    }

    /// Serialize the typed request into the daemon's compatibility envelope.
    /// An absent ID is reserved for client notifications such as approval
    /// responses; it is encoded as JSON `null` to preserve the current wire
    /// contract.
    pub fn to_json_rpc(&self, id: Option<u64>) -> serde_json::Result<serde_json::Value> {
        let (method, params) = match self {
            Self::Chat(params) => ("chat", Some(serde_json::to_value(params)?)),
            Self::Clear => ("clear", None),
            Self::Status => ("status", None),
            Self::Reflect => ("reflect", None),
            Self::ReflectNow => ("reflect_now", None),
            Self::Evolution => ("evolution", None),
            Self::Genome => ("genome", None),
            Self::Sessions => ("sessions", None),
            Self::Resume(params) => ("resume", Some(serde_json::to_value(params)?)),
            Self::Compact => ("compact", None),
            Self::ModelList => ("model_list", None),
            Self::ModeSwitch(params) => ("mode_switch", Some(serde_json::to_value(params)?)),
            Self::PlanApprove => ("plan_approve", None),
            Self::Interrupt(params) => ("interrupt", Some(serde_json::to_value(params)?)),
            Self::HooksList => ("hooks_list", None),
            Self::ApprovalResponse(params) => {
                ("approval_response", Some(serde_json::to_value(params)?))
            }
        };
        serde_json::to_value(JsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION,
            id,
            method,
            params,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientCapabilities {
    pub item_events: bool,
    pub cursors: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitializeParams {
    pub client_version: String,
    pub protocol_versions: Vec<u16>,
    pub capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitializedResult {
    pub protocol_version: u16,
    pub server_capabilities: ClientCapabilities,
    #[schemars(with = "String")]
    pub connection_id: ConnectionId,
    #[schemars(with = "String")]
    pub principal_id: PrincipalId,
    #[schemars(with = "serde_json::Value")]
    pub os_principal: LocalOsPrincipal,
    pub runtime_version: String,
}

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
    Initialize(InitializeParams),
    Initialized,
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
    InitializeResponse(InitializedResult),
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

pub fn negotiate_protocol_version(
    offered_versions: &[u16],
) -> Result<u16, UnsupportedClientVersion> {
    SUPPORTED_CLIENT_PROTOCOL_VERSIONS
        .iter()
        .copied()
        .filter(|version| offered_versions.contains(version))
        .max()
        .ok_or_else(|| UnsupportedClientVersion {
            actual: offered_versions.iter().copied().max().unwrap_or_default(),
            expected: CLIENT_PROTOCOL_VERSION,
        })
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
