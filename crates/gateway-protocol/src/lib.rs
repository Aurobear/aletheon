//! Aletheon Gateway typed protocol contract (CGP-02 owner seam).
//!
//! Versioned commands, queries, events, cursor and error codes that a typed
//! Gateway client and server share.  The server still runs the legacy
//! Executive handlers today; this crate only defines the contract.  It carries
//! opaque client references (`SessionRef`, `TurnRef`, `AgentRef`) and
//! requested preferences — never authoritative effective policy, and never a
//! canonical ID mint.  Caller correlation and `UiOverlayId` live in the client,
//! not here.

use serde::{Deserialize, Serialize};

/// R3 typed command-output surface (closure plan §11).
pub mod command_output;

pub use command_output::{
    validate_version, TypedCommandOutput, TypedCommandOutputEnvelope, TypedCompletion, TypedError,
    TypedStatusProjection, TypedToolLifecycle, TypedUsage, VersionError, COMMAND_OUTPUT_VERSION,
};

/// Negotiated wire protocol version.  Additive only; a bumped minor keeps
/// reading old fields (CGP-02 rollback rule: never re-use a published tag to
/// change semantics).
pub const PROTOCOL_VERSION: u32 = 1;

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

// ── Commands ──────────────────────────────────────────────────────────────

/// Requested session creation.  The server returns a `SessionRef` receipt;
/// the client does not mint a Session ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSessionCreation {
    pub principal_hint: Option<String>,
}

/// Requested session resume by a legacy/reference string.  The Gateway/Runtime
/// atomically resolves and validates the principal binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeSessionReference {
    pub reference: String,
}

/// Submit a prompt to the active (or newly created) session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session: SessionRef,
    pub content: String,
    pub requested_target: RequestedExecutionTarget,
    pub requested_permission: RequestedPermissionMode,
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
}

/// The set of typed commands a client may issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    CreateSession(RequestSessionCreation),
    ResumeSession(ResumeSessionReference),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    SessionSnapshot(SessionSnapshotQuery),
}

// ── Events ────────────────────────────────────────────────────────────────

/// Opaque cursor into the server event stream (sequence within a session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor(pub u64);

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
    SessionCreated { session: SessionRef },
    TurnStarted { session: SessionRef, turn: TurnRef },
    TurnSettled(TurnSettlement),
    Snapshot(SessionSnapshotQuery),
}

// ── Errors ────────────────────────────────────────────────────────────────

/// Typed command/query/event error.  `UnknownSchema`, timeout and EOF are
/// distinct typed failures — a client must fail closed rather than report
/// success on any of these.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("wire version mismatch: server {server}, client {client}")]
    VersionMismatch { server: u32, client: u32 },
    #[error("unknown or malformed schema")]
    UnknownSchema,
    #[error("request timed out")]
    Timeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("server returned an error: {0}")]
    Server(String),
    #[error("request was cancelled")]
    Cancelled,
}
