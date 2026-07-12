//! Capability invoker trait — Phase 5A.
//!
//! The capability invoker is the unified entry point for executing tool/capability
//! calls. It wraps the admission controller: every invocation goes through
//! `admit → execute → settle`.
//!
//! Uses the existing `CapabilityRequest` / `CapabilityResult` types from
//! `crate::include::turn` to avoid type duplication across the codebase.

use crate::include::turn::{CapabilityRequest, CapabilityResult};
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
}
