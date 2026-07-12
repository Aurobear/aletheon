//! BodyRuntime trait — like Linux kernel's device_ops / HAL.
//!
//! BodyRuntime is the execution layer — it actually does things in the world
//! (run shell commands, read files, interact with browsers, control robots).
//! It can refuse execution (permission denied, dangerous command, etc.).

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Capability;
use crate::Context;
use crate::Subsystem;

/// An action to be executed by BodyRuntime.
///
/// Like a syscall — describes what the caller wants done.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Action name (e.g., "shell.execute", "file.read", "browser.navigate").
    pub name: String,
    /// Action parameters (JSON).
    pub parameters: serde_json::Value,
    /// Whether this action requires sandbox isolation.
    pub requires_sandbox: bool,
    /// Maximum time allowed for this action.
    pub timeout: Option<std::time::Duration>,
}

/// Result of an action execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// Whether the action succeeded.
    pub success: bool,
    /// Output content (text, JSON, binary).
    pub output: String,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Execution time in milliseconds.
    pub elapsed_ms: u64,
    /// Whether the output was truncated.
    pub truncated: bool,
    /// Side effects that occurred (files created, processes spawned, etc.).
    pub side_effects: Vec<SideEffect>,
}

/// A side effect produced by an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEffect {
    /// What kind of side effect.
    pub kind: SideEffectKind,
    /// Human-readable description.
    pub description: String,
    /// Whether this side effect is reversible.
    pub reversible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SideEffectKind {
    FileCreated { path: String },
    FileModified { path: String },
    FileDeleted { path: String },
    ProcessSpawned { pid: u32 },
    NetworkRequest { url: String },
    SystemConfigChanged { key: String },
}

/// BodyRuntime trait — the device HAL of Aletheon.
///
/// Like Linux kernel's `file_operations` vtable — each body backend
/// (shell, filesystem, browser, ROS, etc.) implements this trait.
#[async_trait]
pub trait BodyRuntime: Subsystem {
    /// Execute an action in the world.
    async fn execute(&self, action: Action, ctx: &Context) -> Result<ActionResult>;

    /// List all capabilities this body runtime provides.
    fn capabilities(&self) -> &[Capability];

    /// Pre-check: can this action be executed right now?
    ///
    /// Returns Ok(()) if the action is allowed, or an error describing
    /// why it's denied (permission, resource, danger).
    async fn check(&self, action: &Action, ctx: &Context) -> Result<()>;

    /// Get the name of a specific capability (for logging).
    fn capability_for_action(&self, action_name: &str) -> Option<&Capability> {
        self.capabilities().iter().find(|c| c.name == action_name)
    }
}
