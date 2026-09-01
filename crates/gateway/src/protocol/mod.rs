//! Aletheon Gateway typed protocol contract (CGP-02 owner seam).
//!
//! Versioned commands, queries, events, cursor and error codes that a typed
//! Gateway client and server share. The official socket dispatches this typed
//! contract directly. It carries
//! opaque client references (`SessionRef`, `TurnRef`, `AgentRef`) and
//! requested preferences — never authoritative effective policy, and never a
//! canonical ID mint.  Caller correlation and `UiOverlayId` live in the client,
//! not here.

use serde::{Deserialize, Serialize};

/// R3 typed command-output surface (closure plan §11).
pub mod command_output;
pub mod connection;
pub mod conscious_core;
pub mod exec;
pub mod extension;
pub mod legacy_progress;

pub use command_output::{
    validate_version, TypedCommandOutput, TypedCommandOutputEnvelope, TypedCompletion, TypedError,
    TypedStatusProjection, TypedToolLifecycle, TypedUsage, VersionError, COMMAND_OUTPUT_VERSION,
};
pub use extension::{
    ExtensionEnableRequestV1, ExtensionMutationReceiptV1, ExtensionPackageIdRequestV1,
    ExtensionPackagePathRequestV1, McpConnectorManifestV1, McpConnectorTransportV1,
    EXTENSION_PROTOCOL_SCHEMA_V1, MAX_CONNECTOR_REQUEST_TIMEOUT_MS,
};

/// Negotiated wire protocol version.  Additive only; a bumped minor keeps
/// reading old fields (CGP-02 rollback rule: never re-use a published tag to
/// change semantics).
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON-line envelope used by the typed Gateway transport. Request IDs are
/// transport correlation only; they are never aggregate IDs or authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRequest {
    pub version: u32,
    pub request_id: String,
    pub body: WireRequestBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireRequestBody {
    Command(Command),
    Query(Query),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireResponse {
    pub version: u32,
    pub request_id: String,
    pub body: WireResponseBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireResponseBody {
    Command(CommandOutcome),
    Query(serde_json::Value),
    Event(Event),
    Error(ProtocolError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandOutcome {
    Created { session: SessionRef },
    Resumed { session: SessionRef },
    SessionUpdated { session: SessionRef },
    Forked { session: SessionRef },
    ModelUpdated { model: String },
    CollaborationModeUpdated { mode: RequestedCollaborationMode },
    AgentProfileUpdated { profile: String },
    Submitted { turn: TurnRef },
    WorkspaceRestored { outcome: WorkspaceRestoreOutcome },
    TransactionReviewed { outcome: serde_json::Value },
    ExtensionResult { result: serde_json::Value },
    Cancelled,
    ApprovalRecorded,
}

// ── Opaque client references ──────────────────────────────────────────────
// These wrap server-returned strings/UUIDs as opaque refs.  They are NOT
// canonical `SessionId`/`TurnId`/`AgentId` mints: a client never constructs a
// canonical core ID.  Only `Runtime`/`Gateway` decode or mint the canonical
// identity.

/// Opaque server-returned session reference (client-side correlation only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef(pub String);

/// Opaque server-returned turn reference (client-side correlation only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRef(pub String);

/// Opaque server-returned agent reference (client-side correlation only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef(pub String);

// ── Requested preferences (not effective authority) ───────────────────────
// The client submits what it *requests*; the authenticated Gateway/Runtime
// computes the effective permission/workspace/target.  No client-side
// effective-policy derivation is performed here.

/// Requested execution target, as a preference.  The Runtime receipt decides
/// the effective target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestedExecutionTarget {
    #[default]
    Automatic,
    /// Explicitly request a general turn. The host still validates the
    /// authenticated workspace/policy before admitting it.
    General,
    /// Explicit embodied target selected by a trusted presentation edge.
    Robot {
        device_id: String,
        environment: String,
    },
    Runtime(String),
}

/// Requested permission mode.  The host computes effective permission from the
/// authenticated principal + policy; the client never claims effective mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestedPermissionMode {
    #[default]
    Inherit,
    Safe,
    Full,
}

/// Collaboration mode requested by the presentation edge.  The host/runtime
/// remains authoritative for the effective mode and any policy transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestedCollaborationMode {
    Default,
    Plan,
    Auto,
    Sandbox,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetModelRequest {
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCollaborationModeRequest {
    pub mode: RequestedCollaborationMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAgentProfileRequest {
    pub profile: String,
}

/// Invoke a daemon-registered Skill through the authenticated Runtime path.
/// The client supplies only the Skill id, user arguments, and optional
/// session/workspace references; the daemon resolves the trusted descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInvokeRequest {
    pub skill_id: String,
    #[serde(default)]
    pub user_args: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// Execute a user-requested shell command through the daemon's governed
/// capability path. The command is data; the host still performs admission,
/// approval, audit, and terminal projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteShellRequest {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub requested_permission: RequestedPermissionMode,
}

/// Submit a host-governed transaction review decision. The daemon loads the
/// canonical transaction/findings and the client only supplies the requested
/// action plus explicit best-effort rollback acknowledgement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewTransactionRequest {
    pub session: SessionRef,
    pub transaction: String,
    pub action: ::contracts::TransactionReviewAction,
    #[serde(default)]
    pub acknowledge_risk: bool,
}

/// Host-owned extension lifecycle operation. Package inspection/validation
/// remains local; installed-state mutations and reads go through the
/// authenticated daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtensionRequest {
    Install {
        path: String,
        trust_workspace: bool,
        approve_permissions: bool,
    },
    List,
    Show {
        id: String,
    },
    Enable {
        id: String,
        approve_permissions: bool,
    },
    Disable {
        id: String,
    },
    Upgrade {
        path: String,
        trust_workspace: bool,
        approve_permissions: bool,
    },
    Rollback {
        id: String,
    },
    Remove {
        id: String,
    },
    Purge {
        id: String,
    },
    Doctor {
        id: String,
    },
}

/// Restore the daemon-owned workspace checkpoint identified by a logical
/// prompt index.  Filesystem paths and checkpoint identifiers remain host
/// authority; the client supplies only the session reference and index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreWorkspaceCheckpoint {
    pub session: SessionRef,
    pub prompt_index: u64,
}

/// Typed result of a workspace restore.  A non-`Completed` value is an
/// authoritative command result, not a successful-looking text response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceRestoreOutcome {
    Completed,
    IdentityMismatch,
    UnprotectedChangesAbort,
    FsRestoreFailed { detail: String },
    Partial { detail: String },
}

// ── Commands ──────────────────────────────────────────────────────────────

/// Requested session creation.  The server returns a `SessionRef` receipt;
/// the client does not mint a Session ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSessionCreation {
    pub principal_hint: Option<String>,
    /// Requested workspace used by the server-side workspace/session resolver.
    /// It is a preference; the daemon validates and normalizes it before
    /// binding a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// Requested session resume by a legacy/reference string.  The Gateway/Runtime
/// atomically resolves and validates the principal binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeSessionReference {
    pub reference: String,
}

/// Session branch selection is a server-side cursor, not a client-minted ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSessionRequest {
    pub session: SessionRef,
    pub through_sequence: u64,
}

/// Submit a prompt to the active (or newly created) session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session: SessionRef,
    pub content: String,
    /// Requested workspace. The authenticated daemon resolves the effective
    /// workspace; clients never submit an effective policy object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub requested_target: RequestedExecutionTarget,
    pub requested_permission: RequestedPermissionMode,
    /// Optional host-routed Agent runtime requirements. These are requests,
    /// not evidence that a runtime was selected or completed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_agent_runtimes: Vec<String>,
    /// Optional task classification requested by the client. The host parses
    /// and validates it before constructing the effective turn request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_task_kind: Option<String>,
}

/// Cancel the active turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelActiveTurn {
    pub session: SessionRef,
}

/// Submit an approval choice.  A decision here is a user gesture; the Kernel
/// performs the validated approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitApprovalChoice {
    pub session: SessionRef,
    pub choice_id: String,
    pub approved: bool,
    /// Optimistic version of the server-issued approval snapshot.  The
    /// Gateway/approval service, not the client, decides whether this version
    /// is still current.
    #[serde(default)]
    pub version: u64,
    /// Optional rejection explanation. It is request data, never an approval
    /// authority or a replacement for the server-side decision record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The set of typed commands a client may issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    CreateSession(RequestSessionCreation),
    ResumeSession(ResumeSessionReference),
    ClearSession(SessionRef),
    ForkSession(ForkSessionRequest),
    CompactSession(SessionRef),
    SetModel(SetModelRequest),
    SetCollaborationMode(SetCollaborationModeRequest),
    SetAgentProfile(SetAgentProfileRequest),
    InvokeSkill(SkillInvokeRequest),
    ExecuteShell(ExecuteShellRequest),
    ReviewTransaction(ReviewTransactionRequest),
    ManageExtension(ExtensionRequest),
    RestoreWorkspaceCheckpoint(RestoreWorkspaceCheckpoint),
    SubmitPrompt(SubmitPromptRequest),
    CancelActiveTurn(CancelActiveTurn),
    SubmitApproval(SubmitApprovalChoice),
}

// ── Queries ───────────────────────────────────────────────────────────────

/// Read the current session snapshot projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotQuery {
    pub session: SessionRef,
    pub after_cursor: Option<Cursor>,
    /// Request an item-free baseline plus one bounded event page instead of a
    /// full snapshot. This keeps long-session responses below the wire limit.
    #[serde(default)]
    pub paged: bool,
}

/// Read the authenticated daemon skill catalog.  The catalog is a projection
/// of host-owned skill descriptors; it is not a client-side command registry
/// or an authority to enable a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogQuery;

/// Read the authenticated provider/model catalog projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogQuery;

/// Query the authenticated, daemon-owned child-Agent projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCatalogQuery;

/// Query the authenticated agent-profile catalog projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfileCatalogQuery;

/// Read the daemon-owned production health projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatusQuery;

/// Read the daemon-owned memory health projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStatusQuery;

/// Read bounded memory recall through the authenticated memory facade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchQuery {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// Read the daemon-owned core/recent memory snapshot formerly exposed by the
/// `session.memory` compatibility method. The server applies the authenticated
/// session binding and keeps memory state out of the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySnapshotQuery {
    pub session: SessionRef,
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
    #[serde(default = "default_memory_limit")]
    pub limit: u16,
}

/// Read the latest host settlement receipt for one authenticated session and
/// transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionSettlementQuery {
    pub session: SessionRef,
    pub transaction: String,
}

fn default_memory_type() -> String {
    "all".into()
}

const fn default_memory_limit() -> u16 {
    20
}

/// Read the daemon-owned workspace checkpoint projection for one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointListQuery {
    pub session: SessionRef,
    #[serde(default = "default_checkpoint_limit")]
    pub limit: u16,
}

const fn default_checkpoint_limit() -> u16 {
    64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    Health(HealthStatusQuery),
    SessionSnapshot(SessionSnapshotQuery),
    SessionList,
    SkillCatalog(SkillCatalogQuery),
    ModelCatalog(ModelCatalogQuery),
    AgentCatalog(AgentCatalogQuery),
    AgentProfileCatalog(AgentProfileCatalogQuery),
    MemoryStatus(MemoryStatusQuery),
    MemorySearch(MemorySearchQuery),
    MemorySnapshot(MemorySnapshotQuery),
    TransactionSettlement(TransactionSettlementQuery),
    CheckpointList(CheckpointListQuery),
}

// ── Events ────────────────────────────────────────────────────────────────

/// Opaque cursor into the server event stream.
///
/// `sequence` is useful for ordering, while `event_id` binds the cursor to
/// the durable event at that position.  The id is optional on the wire for
/// origin cursors and for rolling upgrades; servers must reject a non-origin
/// cursor that cannot be authenticated against the durable event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cursor {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

impl<'de> Deserialize<'de> for Cursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum WireCursor {
            Legacy(u64),
            Current {
                sequence: u64,
                #[serde(default)]
                event_id: Option<String>,
            },
        }
        match WireCursor::deserialize(deserializer)? {
            WireCursor::Legacy(sequence) => Ok(Self {
                sequence,
                event_id: None,
            }),
            WireCursor::Current { sequence, event_id } => Ok(Self { sequence, event_id }),
        }
    }
}

impl Cursor {
    pub const fn origin() -> Self {
        Self {
            sequence: 0,
            event_id: None,
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::Cursor;

    #[test]
    fn cursor_reads_legacy_sequence_and_new_bound_cursor() {
        let legacy: Cursor = serde_json::from_str("7").unwrap();
        assert_eq!(legacy.sequence, 7);
        assert_eq!(legacy.event_id, None);
        let current: Cursor =
            serde_json::from_str(r#"{"sequence":7,"event_id":"event-7"}"#).unwrap();
        assert_eq!(current.event_id.as_deref(), Some("event-7"));
    }
}

/// Terminal settlement delivered by the server.  Terminal is authoritative
/// here — the client never infers terminal from text/spinner/EOF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSettlement {
    pub turn: TurnRef,
    pub terminal: SettlementTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementTerminal {
    Completed,
    Interrupted,
    Failed { message: String },
}

/// Typed events a client may receive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    SessionCreated {
        session: SessionRef,
    },
    TurnStarted {
        session: SessionRef,
        turn: TurnRef,
    },
    TurnSettled(TurnSettlement),
    Snapshot(SessionSnapshotQuery),
    /// Runtime progress is deliberately separate from public Session/Turn
    /// events. The payload is an immutable presentation observation; it is
    /// never interpreted as lifecycle authority by a client.
    Progress(RuntimeProgressEvent),
    /// A connection-owned approval request emitted by the guarded tool
    /// runner. `choice_id` is opaque and must be submitted back to the same
    /// authenticated connection; it is not a durable ApprovalId mint.
    ApprovalRequested(ApprovalRequestedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProgressEvent {
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequestedEvent {
    pub session: SessionRef,
    pub turn: TurnRef,
    pub choice_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
}

// ── Errors ────────────────────────────────────────────────────────────────

/// Typed command/query/event error.  `UnknownSchema`, timeout and EOF are
/// distinct typed failures — a client must fail closed rather than report
/// success on any of these.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolError {
    #[error("wire version mismatch: server {server}, client {client}")]
    VersionMismatch { server: u32, client: u32 },
    #[error("unknown or malformed schema")]
    UnknownSchema,
    #[error("request timed out")]
    Timeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error(
        "pending event buffer overflowed after dropping {dropped_progress_events} progress events"
    )]
    EventBufferOverflow { dropped_progress_events: u64 },
    #[error("wire frame exceeds the configured limit")]
    FrameTooLarge,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("server returned an error: {0}")]
    Server(String),
    #[error("request was cancelled")]
    Cancelled,
}

pub use conscious_core::{CandidateDisposition, ConsciousCoreSnapshot, InspectorProcessorAck};
