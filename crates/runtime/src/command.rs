//! Runtime typed commands (RA-01).
//!
//! Commands the Runtime accepts.  None carries a new canonical aggregate ID:
//! the Runtime assigns `SessionId`/`TurnId`/`AgentRunId` from its own
//! composition.  Callers pass correlation references and requested
//! preferences only.  These are contracts — the Runtime does not append,
//! spawn, or cut over any legacy writer in this seam.

use crate::ids::{AgentRunId, SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// Create a new session.  No aggregate ID is accepted; the Runtime assigns
/// the canonical `SessionId` and returns it in the receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionCommand {
    /// Optional caller-side correlation reference (no authority).
    pub correlation: Option<String>,
    /// Requested principal hint; effective principal is established by the
    /// authenticated host, never by the caller.
    pub principal_hint: Option<String>,
}

/// Resume a session by a legacy/reference string.  The Runtime resolves and
/// validates the principal binding atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeSessionCommand {
    /// Caller-supplied session reference (opaque; resolved by Runtime).
    pub session_reference: String,
}

/// Start a turn on a session.  `TurnId` is assigned by the Runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartTurnCommand {
    pub session: SessionId,
    pub content: String,
}

/// Spawn a child agent run.  `AgentRunId` is assigned by the Runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentRunCommand {
    pub parent_session: SessionId,
    pub parent_turn: TurnId,
    pub requested_profile: Option<String>,
}

/// Cancel the active turn on a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelTurnCommand {
    pub session: SessionId,
}

/// The set of typed commands the Runtime accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeCommand {
    CreateSession(CreateSessionCommand),
    ResumeSession(ResumeSessionCommand),
    StartTurn(StartTurnCommand),
    SpawnAgentRun(SpawnAgentRunCommand),
    CancelTurn(CancelTurnCommand),
}

/// Runtime-assigned receipt returned after a command is admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub agent_run: Option<AgentRunId>,
    pub generation: Option<crate::ids::Generation>,
}
