//! High-level IPC message types for inter-agent communication.
//!
//! Builds on `agent::Pid` and `ipc_types::AgentMessage` to provide
//! structured, typed messages with signal semantics and fork directives.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::agent::Pid;

// ---------------------------------------------------------------------------
// Signal enum
// ---------------------------------------------------------------------------

/// Control signals that can be sent between agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Signal {
    Abort,
    Pause,
    Resume,
    HealthCheck,
    BudgetWarning,
}

// ---------------------------------------------------------------------------
// MessageKind enum
// ---------------------------------------------------------------------------

/// Discriminator for IPC message semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageKind {
    Task,
    Result,
    Query,
    Response,
    Signal(Signal),
}

// ---------------------------------------------------------------------------
// IpcMessage
// ---------------------------------------------------------------------------

/// Structured IPC message exchanged between agent processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub from: Pid,
    pub to: Pid,
    pub kind: MessageKind,
    pub payload: String,
    pub timestamp_ms: u64,
}

impl IpcMessage {
    /// Create a new IPC message with an automatic timestamp.
    pub fn new(from: Pid, to: Pid, kind: MessageKind, payload: String) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            from,
            to,
            kind,
            payload,
            timestamp_ms,
        }
    }

    /// Convenience constructor for task messages.
    pub fn task(from: Pid, to: Pid, task: String) -> Self {
        Self::new(from, to, MessageKind::Task, task)
    }

    /// Convenience constructor for result messages.
    pub fn result(from: Pid, to: Pid, result: String) -> Self {
        Self::new(from, to, MessageKind::Result, result)
    }

    /// Convenience constructor for signal messages.
    pub fn signal(from: Pid, to: Pid, signal: Signal) -> Self {
        Self::new(from, to, MessageKind::Signal(signal), String::new())
    }
}

// ---------------------------------------------------------------------------
// ForkDirective
// ---------------------------------------------------------------------------

/// Directive for forking a child agent from a parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkDirective {
    pub prompt: String,
    pub inherit_history: bool,
    pub inherit_tools: bool,
    pub budget_ratio: f64,
}

impl Default for ForkDirective {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            inherit_history: true,
            inherit_tools: true,
            budget_ratio: 0.3,
        }
    }
}

// ---------------------------------------------------------------------------
// ForkResult
// ---------------------------------------------------------------------------

/// Outcome of a completed fork.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkResult {
    pub pid: Pid,
    pub parent_pid: Pid,
    pub output: String,
    pub tokens_consumed: u32,
    pub success: bool,
}

// ---------------------------------------------------------------------------
// GroupId
// ---------------------------------------------------------------------------

/// Identifier for a group of cooperating agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub u64);
