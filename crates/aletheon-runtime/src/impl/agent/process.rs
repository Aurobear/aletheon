//! Agent process lifecycle management.
//!
//! Placeholder for the AgentProcess struct and related types.
//! Will be implemented in a follow-up task.

use serde::{Deserialize, Serialize};

/// Configuration for an agent process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProcessConfig {
    /// Unique agent identifier.
    pub id: String,
    /// Maximum tokens per pulse.
    pub max_tokens_per_pulse: u32,
}

/// Lifecycle states of an agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    /// Agent is idle, waiting for a pulse.
    Idle,
    /// Agent is actively executing.
    Running,
    /// Agent has completed its task.
    Completed,
    /// Agent encountered an error.
    Failed,
}

/// An agent process that executes turns within a pulse.
pub struct AgentProcess {
    config: AgentProcessConfig,
    state: AgentState,
}

impl AgentProcess {
    pub fn new(config: AgentProcessConfig) -> Self {
        Self {
            config,
            state: AgentState::Idle,
        }
    }

    /// Get the current state.
    pub fn state(&self) -> AgentState {
        self.state
    }

    /// Get the agent ID.
    pub fn id(&self) -> &str {
        &self.config.id
    }

    /// Set the agent state.
    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }
}
