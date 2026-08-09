//! Runtime typed queries (RA-01).

use crate::ids::{AgentRunId, SessionId};
use serde::{Deserialize, Serialize};

/// Read the canonical session snapshot projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotQuery {
    pub session: SessionId,
    pub after_cursor: Option<u64>,
}

/// Read the terminal settlement of a specific agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunQuery {
    pub agent_run: AgentRunId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeQuery {
    SessionSnapshot(SessionSnapshotQuery),
    AgentRun(AgentRunQuery),
}
