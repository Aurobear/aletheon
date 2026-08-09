//! Runtime typed events (RA-01).
//!
//! Events the Runtime emits.  Terminal is authoritative here: a client/projection
//! must never infer terminal from text, spinner, EOF or socket state — it may
//! only consume a typed `TurnSettled` event.

use crate::ids::{AgentRunId, SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// A turn reached a terminal state.  This is the only terminal source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnTerminal {
    Completed,
    Interrupted,
    Failed { message: String },
}

/// Typed Runtime events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeEvent {
    SessionCreated {
        session: SessionId,
    },
    TurnStarted {
        session: SessionId,
        turn: TurnId,
    },
    TurnSettled {
        session: SessionId,
        turn: TurnId,
        terminal: TurnTerminal,
    },
    AgentRunStarted {
        session: SessionId,
        agent_run: AgentRunId,
    },
    AgentRunSettled {
        session: SessionId,
        agent_run: AgentRunId,
        terminal: TurnTerminal,
    },
}
