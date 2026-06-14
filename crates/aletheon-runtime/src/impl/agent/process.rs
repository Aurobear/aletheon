//! Agent process lifecycle management.
//!
//! Placeholder for the AgentProcess struct and related types.
//! Will be implemented in a follow-up task.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use aletheon_abi::agent::Pid;
use aletheon_abi::IpcMessage;

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
    pub pid: Pid,
    config: AgentProcessConfig,
    state: AgentState,
    pub inbox: Option<mpsc::Receiver<IpcMessage>>,
    pub last_heartbeat_ms: AtomicU64,
}

impl AgentProcess {
    pub fn new(config: AgentProcessConfig) -> Self {
        Self {
            pid: Pid::new(),
            config,
            state: AgentState::Idle,
            inbox: None,
            last_heartbeat_ms: AtomicU64::new(0),
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

    /// Record the current wall-clock time as the last heartbeat.
    pub fn touch_heartbeat(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_heartbeat_ms.store(now, Ordering::Relaxed);
    }
}
