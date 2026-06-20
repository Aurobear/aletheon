//! Extended hook types for the hooks system.
//!
//! Complements the existing `hook.rs` types with configuration
//! and event bus integration types.

use serde::{Deserialize, Serialize};
use crate::hook::HookPoint;

/// Hook execution type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    /// Spawn a child process. Environment variables injected.
    Command,
    /// Inject as system message into the conversation.
    Prompt,
    /// Emit to the event bus.
    Event,
}

/// Hook configuration (from config.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Which lifecycle point this hook fires at.
    pub point: HookPoint,
    /// How to execute the hook.
    pub hook_type: HookType,
    /// For Command hooks: the shell command to run.
    pub command: Option<String>,
    /// For Prompt hooks: the text to inject.
    pub prompt: Option<String>,
    /// For Event hooks: the event type to emit.
    pub event_type: Option<String>,
    /// Timeout in milliseconds (default: 5000).
    pub timeout_ms: Option<u64>,
}

/// Result returned by a command hook process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHookResult {
    /// Whether the hook wants to modify the original action.
    pub modify: bool,
    /// Modification data (only meaningful when modify=true).
    pub data: Option<serde_json::Value>,
    /// Optional message to inject into conversation.
    pub inject_message: Option<String>,
    /// Whether to block the original action.
    pub block: bool,
    /// Reason for blocking (only meaningful when block=true).
    pub block_reason: Option<String>,
}
