//! Core tool types and trait definitions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AgentId, CapabilityScope, PrincipalId, ProcessId, ThreadId, TurnId, WorkspacePolicy};

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
    /// Optional canonical turn stream supplied by the trusted Executive path.
    /// Tool/model input cannot create or replace this sender.
    pub turn_event_sender: Option<crate::ipc::TurnEventSender>,
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
    /// Kernel-granted resource scope for this exact capability invocation.
    pub granted_scope: CapabilityScope,
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

/// Neutral, protocol-stable description of a structured patch result. Fabric
/// owns the transport contract while Corpus remains free to own patch parsing
/// and filesystem application.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchDelta {
    pub applied: Vec<PatchDeltaApplied>,
    pub failed: Vec<PatchDeltaFailed>,
    pub files_changed: Vec<PatchDeltaFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchDeltaApplied {
    pub operation: String,
    pub path: String,
    pub hunks_applied: Option<usize>,
    pub bytes_written: Option<u64>,
    pub moved_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchDeltaFailed {
    pub operation: String,
    pub path: String,
    pub error: String,
    pub hunks_applied_before_failure: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchDeltaFileChange {
    pub path: String,
    pub change_type: String,
    pub hunks_applied: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResultMeta {
    pub execution_time_ms: u64,
    pub truncated: bool,
    /// Structured filesystem delta produced by an `apply_patch` invocation.
    /// Fabric owns this neutral DTO so it does not depend on Corpus patch types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_delta: Option<PatchDelta>,
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

/// Host-authored execution identity for structured tools.
///
/// This value is obtained from the registered `Tool` implementation, never
/// from model-provided JSON input. Transport implementations must continue to
/// treat `input` and this descriptor as separate trust domains.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolExecutionDescriptor {
    EbpfCompile,
    KernelBuild,
    ModuleBuild,
    ModuleLoad,
    Script { canonical_path: std::path::PathBuf },
}

impl std::fmt::Debug for ToolExecutionDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EbpfCompile => formatter.write_str("EbpfCompile"),
            Self::KernelBuild => formatter.write_str("KernelBuild"),
            Self::ModuleBuild => formatter.write_str("ModuleBuild"),
            Self::ModuleLoad => formatter.write_str("ModuleLoad"),
            Self::Script { .. } => formatter
                .debug_struct("Script")
                .field("canonical_path", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Canonical Tool trait. See shared/traits.md.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn permission_level(&self) -> PermissionLevel;

    /// Trusted, host-only execution identity for isolated structured dispatch.
    /// Read-only and legacy tools inherit `None` and remain unaffected.
    fn execution_descriptor(&self) -> Option<ToolExecutionDescriptor> {
        None
    }
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;

    /// Execute through the additive G2 streaming contract. Legacy tools inherit
    /// this terminal-only adapter and therefore require no implementation
    /// changes. The terminal remains subject to Executive settlement.
    async fn execute_streaming(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
        sink: &mut crate::types::tool_stream::ToolEventSink,
    ) {
        sink.terminal(Ok(self.execute(input, ctx).await)).await;
    }

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

    #[derive(Clone)]
    struct LegacyTool;

    #[async_trait]
    impl Tool for LegacyTool {
        fn name(&self) -> &str {
            "legacy"
        }

        fn description(&self) -> &str {
            "legacy terminal-only tool"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::L0
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult {
                content: "legacy-result".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            }
        }

        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(self.clone())
        }
    }

    struct FixedClock;

    impl crate::Clock for FixedClock {
        fn wall_now(&self) -> crate::WallTime {
            crate::WallTime(0)
        }

        fn mono_now(&self) -> crate::MonoTime {
            crate::MonoTime(0)
        }
    }

    #[tokio::test]
    async fn legacy_tool_streaming_adapter_emits_exactly_one_terminal() {
        let (mut sink, mut rx) = crate::types::tool_stream::tool_event_channel();
        LegacyTool
            .execute_streaming(
                serde_json::json!({}),
                &ToolContext {
                    agent: None,
                    approval_authority: None,
                    working_dir: std::env::temp_dir(),
                    session_id: "test".into(),
                    clock: Arc::new(FixedClock),
                    turn_event_sender: None,
                },
                &mut sink,
            )
            .await;
        drop(sink);

        let terminal = rx.recv().await.expect("terminal event");
        assert!(matches!(
            terminal,
            crate::types::tool_stream::ToolExecutionEvent::Terminal(Ok(ToolResult {
                content,
                is_error: false,
                ..
            })) if content == "legacy-result"
        ));
        assert!(rx.recv().await.is_none(), "adapter must emit one terminal");
    }

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

    #[test]
    fn execution_descriptor_serializes_as_a_separate_typed_contract() {
        let descriptor = ToolExecutionDescriptor::ModuleBuild;
        let encoded = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(encoded, serde_json::json!({"kind": "module_build"}));
        assert_eq!(
            serde_json::from_value::<ToolExecutionDescriptor>(encoded).unwrap(),
            descriptor
        );
    }

    #[test]
    fn script_descriptor_debug_redacts_host_path() {
        let descriptor = ToolExecutionDescriptor::Script {
            canonical_path: "/trusted/host/secret/tool.sh".into(),
        };
        let debug = format!("{descriptor:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret/tool.sh"));
    }

    #[test]
    fn descriptor_rejects_model_supplied_extra_fields() {
        // Use a kind that does not exist in the enum — model-supplied
        // descriptors must not deserialize into any valid variant.
        let injected = serde_json::json!({
            "kind": "model_controlled",
            "canonical_path": "/tmp/model-controlled"
        });
        assert!(serde_json::from_value::<ToolExecutionDescriptor>(injected).is_err());
        assert!(LegacyTool.execution_descriptor().is_none());
    }
}
