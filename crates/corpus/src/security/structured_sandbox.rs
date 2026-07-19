//! Transport port for structured tools whose strategy requires isolation.

use async_trait::async_trait;
use fabric::tool::{ToolContext, ToolExecutionDescriptor, ToolResult};
use fabric::SandboxConfig;

/// Executes a structured tool through an isolated filesystem-capable owner
/// (for example the execd). The runner deliberately has no in-process
/// fallback when this port is required but unavailable.
#[async_trait]
pub trait StructuredToolSandbox: Send + Sync {
    /// Canonical backend identity persisted in guarded execution audit records.
    fn backend_name(&self) -> &'static str {
        "structured-sandbox"
    }

    /// Reports whether this transport implements the named tool's complete
    /// structured contract. The conservative default describes the standard
    /// filesystem mutation contract; transports must opt in explicitly as
    /// more structured tools gain isolated implementations.
    fn supports_tool(&self, tool_name: &str) -> bool {
        matches!(tool_name, "file_write" | "apply_patch")
    }

    async fn execute(
        &self,
        tool_name: &str,
        descriptor: Option<&ToolExecutionDescriptor>,
        input: serde_json::Value,
        context: &ToolContext,
        sandbox: &SandboxConfig,
    ) -> Result<ToolResult, String>;
}
