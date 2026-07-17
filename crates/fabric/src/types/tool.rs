//! Core tool types and trait definitions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AgentId, PrincipalId, ProcessId, ThreadId, TurnId, WorkspacePolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolContext {
    pub caller_root_agent_id: AgentId,
    pub parent_agent_id: AgentId,
    pub parent_process_id: ProcessId,
}

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
    pub agent: Option<AgentToolContext>,
    /// Exact authenticated approval authority, when supplied by the governed
    /// capability path. Legacy contexts are explicitly `None` and fail closed
    /// if an approval is required.
    pub approval_authority: Option<ToolApprovalAuthority>,
    // Compatibility fields used by existing tool implementations. Approval
    // ownership must use the typed fields above.
    pub working_dir: std::path::PathBuf,
    pub session_id: String,
    pub clock: Arc<dyn crate::Clock>,
}

impl ToolContext {
    /// Materialize the effective workspace without changing the typed approval
    /// authority contract. Governed calls retain all resolved writable roots;
    /// legacy contexts are confined to their canonical working directory.
    pub fn effective_workspace_policy(&self) -> Result<WorkspacePolicy, String> {
        if let Some(authority) = &self.approval_authority {
            return Ok(authority.workspace.clone());
        }
        let cwd = std::fs::canonicalize(&self.working_dir).map_err(|error| {
            format!(
                "invalid tool working directory '{}': {error}",
                self.working_dir.display()
            )
        })?;
        WorkspacePolicy::from_resolved_roots(cwd, Vec::new())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolApprovalAuthority {
    pub principal_id: PrincipalId,
    pub connection_id: crate::ConnectionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub call_id: String,
    pub workspace: WorkspacePolicy,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApprovalOwner {
    pub principal_id: PrincipalId,
    pub thread_id: ThreadId,
}

impl ApprovalOwner {
    pub fn new(principal_id: PrincipalId, thread_id: ThreadId) -> Self {
        Self {
            principal_id,
            thread_id,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PendingApprovalKey {
    pub owner: ApprovalOwner,
    pub turn_id: TurnId,
    pub call_id: String,
    pub approval_id: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ThreadGrantKey {
    pub owner: ApprovalOwner,
    pub tool: String,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ToolExposure {
    /// Always visible to model and searchable
    #[default]
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

/// A snapshot of a file before it was modified.
#[derive(Debug, Clone)]
pub struct FileSnap {
    pub path: std::path::PathBuf,
    /// File content at capture time. `None` means the file did not exist.
    pub content: Option<String>,
}

impl FileSnap {
    /// Capture the current state of a file.
    pub fn capture(path: &std::path::Path) -> std::io::Result<Self> {
        let content = if path.exists() {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };
        Ok(Self {
            path: path.to_path_buf(),
            content,
        })
    }

    /// Restore this snapshot to disk.
    pub fn restore(&self) -> std::io::Result<()> {
        match &self.content {
            Some(content) => {
                if let Some(parent) = self.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&self.path, content)?;
            }
            None => {
                if self.path.exists() {
                    std::fs::remove_file(&self.path)?;
                }
            }
        }
        Ok(())
    }
}

/// Tools that can preview their change without touching disk.
///
/// Used by the checkpoint system to capture file state before edits.
/// Only edit/write tools implement this. Bash does not.
#[async_trait]
pub trait Previewer: Tool {
    /// Preview the file change this tool would make.
    /// Returns None if the tool can't preview (e.g., bash).
    fn preview(&self, args: &serde_json::Value) -> Option<FileSnap>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_default_is_direct() {
        assert_eq!(ToolExposure::default(), ToolExposure::Direct);
    }

    #[test]
    fn filter_hidden_tools() {
        let exposures = [
            ToolExposure::Direct,
            ToolExposure::Hidden,
            ToolExposure::Deferred,
            ToolExposure::Hidden,
        ];
        let visible: Vec<_> = exposures
            .iter()
            .filter(|e| **e != ToolExposure::Hidden)
            .collect();
        assert_eq!(visible.len(), 2);
        assert_eq!(*visible[0], ToolExposure::Direct);
        assert_eq!(*visible[1], ToolExposure::Deferred);
    }

    #[test]
    fn keep_direct_tools() {
        let exposures = [
            ToolExposure::Direct,
            ToolExposure::Deferred,
            ToolExposure::Direct,
            ToolExposure::Hidden,
        ];
        let direct: Vec<_> = exposures
            .iter()
            .filter(|e| **e == ToolExposure::Direct)
            .collect();
        assert_eq!(direct.len(), 2);
    }
}
