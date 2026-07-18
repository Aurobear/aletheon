//! Capability invoker trait — Phase 5A.
//!
//! The capability invoker is the unified entry point for executing tool/capability
//! calls. It wraps the admission controller: every invocation goes through
//! `admit → execute → settle`.
//!
//! Uses the existing `CapabilityRequest` / `CapabilityResult` types from
//! `crate::include::turn` to avoid type duplication across the codebase.

use crate::include::turn::{CapabilityRequest, CapabilityResult};
use crate::types::tool::{ToolResult, ToolResultMeta};
use crate::types::tool_stream::ToolEventSink;
use async_trait::async_trait;

/// Unified entry point for capability execution.
///
/// All tool invocations from the LLM (bash, file read/write, etc.) MUST go
/// through `invoke()`. Direct calls to tool implementations are forbidden
/// because they bypass admission, permit checks, and audit.
#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    /// Invoke a capability with full admission + execution + settlement.
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult;

    /// Additive streaming entry point. Implementations that do not expose
    /// incremental progress retain byte-equivalent invocation behavior and
    /// publish only a terminal adapter event.
    async fn invoke_streaming(
        &self,
        request: CapabilityRequest,
        sink: &mut ToolEventSink,
    ) -> CapabilityResult {
        let result = self.invoke(request).await;
        sink.terminal(Ok(ToolResult {
            content: result.output.clone(),
            is_error: result.is_error,
            metadata: ToolResultMeta {
                execution_time_ms: result.usage.wall_time_ms,
                truncated: false,
                patch_delta: result.patch_delta.clone(),
            },
        }))
        .await;
        result
    }
}
