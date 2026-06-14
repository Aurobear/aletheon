//! Core tool types and trait definitions.
//!
//! Core tool types and trait definitions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Permission level for tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Read-only, no side effects
    L0,
    /// Write within sandbox
    L1,
    /// System-level changes
    L2,
    /// Destructive / irreversible
    L3,
}

/// Execution context passed to tools.
pub struct ToolContext {
    pub working_dir: std::path::PathBuf,
    pub session_id: String,
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub metadata: ToolResultMeta,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResultMeta {
    pub execution_time_ms: u64,
    pub truncated: bool,
}

/// Visibility tier for tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolExposure {
    /// Always visible to model and searchable
    Direct,
    /// Only visible after explicit user request
    Deferred,
    /// Visible to model in code mode only
    DirectModelOnly,
    /// Never exposed (internal-only)
    Hidden,
}

impl ToolExposure {
    pub fn is_visible_to_model(&self) -> bool {
        matches!(self, Self::Direct | Self::DirectModelOnly)
    }

    pub fn is_searchable(&self) -> bool {
        matches!(self, Self::Direct | Self::Deferred)
    }

    pub fn is_code_mode_visible(&self) -> bool {
        matches!(self, Self::Direct | Self::DirectModelOnly)
    }
}

/// Concurrency class for parallel tool execution scheduling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConcurrencyClass {
    /// Read-only, safe to run concurrently
    ReadOnly,
    /// Writes to specific paths, serialized per-path
    Write { paths: Vec<std::path::PathBuf> },
    /// Side effects, always serialized
    SideEffect,
}

/// Canonical Tool trait. See shared/traits.md.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn permission_level(&self) -> PermissionLevel;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;

    /// Clone this tool into a `Box<dyn Tool>`. Required for agent config loading
    /// where tools must be duplicated across agents.
    fn boxed_clone(&self) -> Box<dyn Tool>;

    /// Visibility tier for this tool. Default is `Direct` (always visible).
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }

    /// Concurrency class for parallel execution scheduling.
    /// Default is `SideEffect` (always serialized) for safety.
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }
}
