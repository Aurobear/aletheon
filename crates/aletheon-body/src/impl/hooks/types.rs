//! Core types for the lifecycle hooks system.

use serde::{Deserialize, Serialize};

/// Lifecycle events that can trigger hooks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// Before a tool executes.
    PreToolUse,
    /// After a tool executes.
    PostToolUse,
    /// When a session begins.
    SessionStart,
    /// When the user sends a message.
    UserPromptSubmit,
    /// Before context compaction.
    PreCompact,
    /// After context compaction.
    PostCompact,
    /// When the session ends.
    Stop,
}

/// Payload sent to a hook via stdin as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPayload {
    pub event: HookEvent,
    pub session_id: String,
    pub timestamp: String,
    /// Event-specific data.
    pub data: serde_json::Value,
}

/// Response expected from a hook on stdout as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponse {
    /// `false` = block the action.
    pub proceed: bool,
    /// Optional modified payload for the action.
    pub modified: Option<serde_json::Value>,
    /// Feedback message for the agent.
    pub message: Option<String>,
}

/// A single registered hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub name: String,
    pub event: HookEvent,
    /// Shell command to execute.
    pub command: String,
    /// Maximum execution time in milliseconds.
    pub timeout_ms: u64,
}

impl Default for HookResponse {
    fn default() -> Self {
        Self {
            proceed: true,
            modified: None,
            message: None,
        }
    }
}
