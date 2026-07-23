//! Typed client bindings derived from the versioned Fabric protocol model.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ui_event::{CollaborationMode, InterruptReason},
    AgentSnapshot, ApprovalSnapshot, ConnectionId, ItemRecord, LocalOsPrincipal, OperationId,
    PrincipalId, SessionId, ThreadId, TurnId, TurnStop, TurnTerminalStatus, WorkspacePolicy,
};

pub const CLIENT_PROTOCOL_VERSION: u16 = 1;
pub const JSON_RPC_VERSION: &str = "2.0";
const SUPPORTED_CLIENT_PROTOCOL_VERSIONS: &[u16] = &[CLIENT_PROTOCOL_VERSION];

/// Typed compatibility requests for the daemon's line-delimited JSON-RPC
/// transport. Method names and parameter field names live in Fabric rather
/// than being re-created by each Interact entry point.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
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
    AgentProfileList,
    AgentProfileSet(AgentProfileSetParams),
    SkillsList,
    ModeSwitch(ModeSwitchParams),
    PlanApprove,
    Cancel,
    Interrupt(InterruptParams),
    HooksList,
    DaemonShutdown,
    SessionNew,
    ApprovalResponse(ApprovalResponseParams),
    MemoryAdd(MemoryAddParams),
    MemoryList(MemoryListParams),
    MemorySearch(MemorySearchParams),
    MemoryShow(NumericIdParams),
    MemoryForget(MemoryForgetParams),
    MemoryPin(NumericIdParams),
    MemoryUnpin(NumericIdParams),
    GoalSet(GoalSetParams),
    GoalShow(NumericIdParams),
    GoalStatus(GoalStatusParams),
    GoalResume,
    WorkflowSave(WorkflowSaveParams),
    WorkflowLoad(NameParams),
    WorkflowList,
    WorkflowDelete(NameParams),
    WorkflowRun(NameParams),
    DebugTopics,
    DebugSubscribe(DebugSubscribeParams),
    DebugNodeInfo,
    DebugBagStart(DebugBagStartParams),
    DebugBagStop(DebugBagStopParams),
    DebugBagReplay(DebugBagReplayParams),
    DebugPerf,
    DebugTraceStart(DebugTraceStartParams),
    DebugTraceStop,
    DebugTraceStatus,
    Health,
    DebugHealth,
    DebugNodes,
    DebugParamGet(DebugParamGetParams),
    DebugParamList,
    DebugTopology,
    DebugGraph,
    DebugLogSubscribe(DebugLogSubscribeParams),
    SessionResume(ResumeParams),
    SessionFork(SessionForkParams),
    SessionInterrupt(SessionInterruptParams),
    SessionReplay(SessionReplayParams),
    HostComputer(ComputerHostParams),
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
pub struct SessionForkParams {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub through_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SessionInterruptParams {
    #[schemars(with = "String")]
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SessionReplayParams {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub after_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComputerHostParams {
    pub operation: String,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ModeSwitchParams {
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AgentProfileSetParams {
    pub profile: String,
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

impl TransientApprovalDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::ApproveForSession => "approve_for_session",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ApprovalResponseParams {
    pub approval_id: String,
    pub decision: TransientApprovalDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MemoryAddParams {
    pub content: String,
    pub scope: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MemoryListParams {
    pub scope: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MemorySearchParams {
    pub query: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NumericIdParams {
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MemoryForgetParams {
    pub id: i64,
    pub hard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GoalSetParams {
    pub description: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GoalStatusParams {
    pub id: Option<i64>,
    pub status: Option<String>,
    pub filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WorkflowSaveParams {
    pub name: String,
    #[schemars(with = "serde_json::Value")]
    pub def: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NameParams {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugSubscribeParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracepoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugBagStartParams {
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugBagStopParams {
    pub recording_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DebugBagReplayParams {
    pub path: String,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugTraceStartParams {
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugParamGetParams {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DebugLogSubscribeParams {
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: Option<u64>,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct EmptyParams {}

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

    pub fn agent_profile_set(profile: impl Into<String>) -> Self {
        Self::AgentProfileSet(AgentProfileSetParams {
            profile: profile.into(),
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

    pub fn memory_add(
        content: impl Into<String>,
        scope: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Self::MemoryAdd(MemoryAddParams {
            content: content.into(),
            scope: scope.into(),
            subject: subject.into(),
        })
    }

    pub fn memory_list(scope: Option<String>, all: bool) -> Self {
        Self::MemoryList(MemoryListParams { scope, all })
    }

    pub fn memory_search(query: impl Into<String>, scope: Option<String>) -> Self {
        Self::MemorySearch(MemorySearchParams {
            query: query.into(),
            scope,
        })
    }

    pub fn memory_show(id: i64) -> Self {
        Self::MemoryShow(NumericIdParams { id })
    }

    pub fn memory_forget(id: i64, hard: bool) -> Self {
        Self::MemoryForget(MemoryForgetParams { id, hard })
    }

    pub fn memory_pin(id: i64) -> Self {
        Self::MemoryPin(NumericIdParams { id })
    }

    pub fn memory_unpin(id: i64) -> Self {
        Self::MemoryUnpin(NumericIdParams { id })
    }

    pub fn goal_set(description: impl Into<String>, scope: impl Into<String>) -> Self {
        Self::GoalSet(GoalSetParams {
            description: description.into(),
            scope: scope.into(),
        })
    }

    pub fn goal_status(id: Option<i64>, status: Option<String>, filter: Option<String>) -> Self {
        Self::GoalStatus(GoalStatusParams { id, status, filter })
    }

    pub fn goal_show(id: i64) -> Self {
        Self::GoalShow(NumericIdParams { id })
    }

    pub fn workflow_save(name: impl Into<String>, def: serde_json::Value) -> Self {
        Self::WorkflowSave(WorkflowSaveParams {
            name: name.into(),
            def,
        })
    }

    pub fn workflow_load(name: impl Into<String>) -> Self {
        Self::WorkflowLoad(NameParams { name: name.into() })
    }

    pub fn workflow_delete(name: impl Into<String>) -> Self {
        Self::WorkflowDelete(NameParams { name: name.into() })
    }

    pub fn workflow_run(name: impl Into<String>) -> Self {
        Self::WorkflowRun(NameParams { name: name.into() })
    }

    pub fn debug_subscribe(
        level: Option<String>,
        module: Option<String>,
        tracepoint: Option<String>,
    ) -> Self {
        Self::DebugSubscribe(DebugSubscribeParams {
            level,
            module,
            tracepoint,
        })
    }

    pub fn debug_bag_start(
        level: impl Into<String>,
        path: Option<String>,
        module: Option<String>,
    ) -> Self {
        Self::DebugBagStart(DebugBagStartParams {
            level: level.into(),
            path,
            module,
        })
    }

    pub fn debug_bag_stop(recording_id: impl Into<String>) -> Self {
        Self::DebugBagStop(DebugBagStopParams {
            recording_id: recording_id.into(),
        })
    }

    pub fn debug_bag_replay(path: impl Into<String>, speed: f64) -> Self {
        Self::DebugBagReplay(DebugBagReplayParams {
            path: path.into(),
            speed,
        })
    }

    pub fn debug_trace_start(level: impl Into<String>, module: Option<String>) -> Self {
        Self::DebugTraceStart(DebugTraceStartParams {
            level: level.into(),
            module,
        })
    }

    pub fn debug_param_get(key: impl Into<String>) -> Self {
        Self::DebugParamGet(DebugParamGetParams { key: key.into() })
    }

    pub fn debug_log_subscribe(level: impl Into<String>, module: Option<String>) -> Self {
        Self::DebugLogSubscribe(DebugLogSubscribeParams {
            level: level.into(),
            module,
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
            Self::AgentProfileList => ("agent.profile.list", None),
            Self::AgentProfileSet(params) => {
                ("agent.profile.set", Some(serde_json::to_value(params)?))
            }
            Self::SkillsList => ("skills.list", None),
            Self::ModeSwitch(params) => ("mode_switch", Some(serde_json::to_value(params)?)),
            Self::PlanApprove => ("plan_approve", None),
            Self::Cancel => ("cancel", None),
            Self::Interrupt(params) => ("interrupt", Some(serde_json::to_value(params)?)),
            Self::HooksList => ("hooks_list", None),
            Self::DaemonShutdown => (
                "daemon.shutdown",
                Some(serde_json::to_value(EmptyParams {})?),
            ),
            Self::SessionNew => ("session.new", None),
            Self::ApprovalResponse(params) => {
                ("approval_response", Some(serde_json::to_value(params)?))
            }
            Self::MemoryAdd(params) => ("memory.add", Some(serde_json::to_value(params)?)),
            Self::MemoryList(params) => ("memory.list", Some(serde_json::to_value(params)?)),
            Self::MemorySearch(params) => ("memory.search", Some(serde_json::to_value(params)?)),
            Self::MemoryShow(params) => ("memory.show", Some(serde_json::to_value(params)?)),
            Self::MemoryForget(params) => ("memory.forget", Some(serde_json::to_value(params)?)),
            Self::MemoryPin(params) => ("memory.pin", Some(serde_json::to_value(params)?)),
            Self::MemoryUnpin(params) => ("memory.unpin", Some(serde_json::to_value(params)?)),
            Self::GoalSet(params) => ("goal.set", Some(serde_json::to_value(params)?)),
            Self::GoalShow(params) => ("goal.show", Some(serde_json::to_value(params)?)),
            Self::GoalStatus(params) => ("goal.status", Some(serde_json::to_value(params)?)),
            Self::GoalResume => ("goal.resume", Some(serde_json::to_value(EmptyParams {})?)),
            Self::WorkflowSave(params) => ("workflow.save", Some(serde_json::to_value(params)?)),
            Self::WorkflowLoad(params) => ("workflow.load", Some(serde_json::to_value(params)?)),
            Self::WorkflowList => ("workflow.list", Some(serde_json::to_value(EmptyParams {})?)),
            Self::WorkflowDelete(params) => {
                ("workflow.delete", Some(serde_json::to_value(params)?))
            }
            Self::WorkflowRun(params) => ("workflow.run", Some(serde_json::to_value(params)?)),
            Self::DebugTopics => empty_params("debug.topics")?,
            Self::DebugSubscribe(params) => {
                ("debug.subscribe", Some(serde_json::to_value(params)?))
            }
            Self::DebugNodeInfo => empty_params("debug.node_info")?,
            Self::DebugBagStart(params) => ("debug.bag_start", Some(serde_json::to_value(params)?)),
            Self::DebugBagStop(params) => ("debug.bag_stop", Some(serde_json::to_value(params)?)),
            Self::DebugBagReplay(params) => {
                ("debug.bag_replay", Some(serde_json::to_value(params)?))
            }
            Self::DebugPerf => empty_params("debug.perf")?,
            Self::DebugTraceStart(params) => {
                ("debug.trace_start", Some(serde_json::to_value(params)?))
            }
            Self::DebugTraceStop => empty_params("debug.trace_stop")?,
            Self::DebugTraceStatus => empty_params("debug.trace_status")?,
            Self::Health => empty_params("health")?,
            Self::DebugHealth => empty_params("debug.health")?,
            Self::DebugNodes => empty_params("debug.nodes")?,
            Self::DebugParamGet(params) => ("debug.param_get", Some(serde_json::to_value(params)?)),
            Self::DebugParamList => empty_params("debug.param_list")?,
            Self::DebugTopology => empty_params("debug.topology")?,
            Self::DebugGraph => empty_params("debug.graph")?,
            Self::DebugLogSubscribe(params) => {
                ("debug.log_subscribe", Some(serde_json::to_value(params)?))
            }
            Self::SessionResume(params) => ("session.resume", Some(serde_json::to_value(params)?)),
            Self::SessionFork(params) => ("session.fork", Some(serde_json::to_value(params)?)),
            Self::SessionInterrupt(params) => {
                ("session.interrupt", Some(serde_json::to_value(params)?))
            }
            Self::SessionReplay(params) => ("session.replay", Some(serde_json::to_value(params)?)),
            Self::HostComputer(params) => ("host.computer", Some(serde_json::to_value(params)?)),
        };
        serde_json::to_value(JsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION,
            id,
            method,
            params,
        })
    }
}

fn empty_params(
    method: &'static str,
) -> serde_json::Result<(&'static str, Option<serde_json::Value>)> {
    Ok((method, Some(serde_json::to_value(EmptyParams {})?)))
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

/// Start a turn on an explicitly named thread. Workspace authority is supplied
/// independently and must be bound/verified by the host; it is never used to
/// infer the thread identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatRequest {
    pub thread_id: ThreadId,
    pub message: String,
    pub working_dir: std::path::PathBuf,
    #[serde(default)]
    pub additional_writable_roots: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionRequest {
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub operation_id: OperationId,
    #[schemars(with = "String")]
    pub approval_id: crate::ApprovalId,
    pub version: u64,
    pub decision: ApprovalDecisionRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CancelRequest {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub operation_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientRequest {
    Initialize(InitializeParams),
    Initialized,
    Snapshot(SnapshotRequest),
    Subscribe(EventSubscription),
    Chat(ChatRequest),
    Approval(ApprovalRequest),
    Cancel(CancelRequest),
}

impl ClientRequest {
    pub fn to_json_rpc(&self, id: u64) -> serde_json::Result<serde_json::Value> {
        let method = match self {
            Self::Initialize(_) => "initialize",
            Self::Initialized => "initialized",
            Self::Snapshot(_) => "session.snapshot",
            Self::Subscribe(_) => "session.subscribe",
            Self::Chat(_) => "thread.chat",
            Self::Approval(_) => "turn.approval",
            Self::Cancel(_) => "turn.cancel",
        };
        serde_json::to_value(JsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION,
            id: Some(id),
            method,
            params: Some(serde_json::to_value(ClientMessage::v1(self.clone()))?),
        })
    }
}

pub fn session_notification_to_json(
    notification: &crate::SessionNotification,
) -> serde_json::Result<serde_json::Value> {
    #[derive(Serialize)]
    struct JsonRpcNotification {
        jsonrpc: &'static str,
        method: &'static str,
        params: serde_json::Value,
    }
    serde_json::to_value(JsonRpcNotification {
        jsonrpc: JSON_RPC_VERSION,
        method: "session.notification",
        params: serde_json::to_value(notification)?,
    })
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

/// Structured terminal failure carried by the versioned protocol. `code` is
/// stable for clients; `message` remains human-readable diagnostic context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnCompletionError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

/// Bounded turn-level usage projection. Additive fields default to zero so a
/// current client can decode terminal events emitted by an older daemon.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnCompletionUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub elapsed_ms: u64,
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
    CommandCompleted {
        command: String,
        thread_id: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<OperationId>,
        #[schemars(with = "serde_json::Value")]
        detail: serde_json::Value,
    },
    Failed {
        cursor: Option<EventCursor>,
        message: String,
    },
    TurnStarted {
        thread_id: ThreadId,
        turn_id: TurnId,
        operation_id: OperationId,
        iteration: u32,
    },
    TurnCompleted {
        thread_id: ThreadId,
        turn_id: TurnId,
        operation_id: OperationId,
        /// Compatibility field retained for protocol-v1 decoders.
        stop: TurnStop,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<TurnTerminalStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<TurnCompletionError>,
        #[serde(default)]
        retryable: bool,
        #[serde(default)]
        usage: TurnCompletionUsage,
    },
    TurnStopped {
        thread_id: ThreadId,
        turn_id: TurnId,
        operation_id: OperationId,
        reason: TurnStop,
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
