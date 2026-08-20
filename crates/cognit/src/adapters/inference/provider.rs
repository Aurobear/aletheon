//! Re-exports of the canonical LLM provider trait and types.
//!
//! These items now live in `contracts` (RFC-018 Phase 4, resolves D4) since they
//! are a shared client abstraction, not cognit-specific implementation. This
//! Cognit uses the shared contract internally and exposes it through the stable
//! `cognit::inference::provider` facade. Provider transports stay private.
pub use ::contracts::{
    canonicalize_tool_definitions, CacheTelemetry, InferenceCapabilities, InferenceUsage,
    LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk,
};

/// Tool definition sent to the LLM.
pub use ::contracts::ToolDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceFailureKind {
    Transient,
    ContextOverflow,
    Terminal,
}

#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct InferenceFailure {
    pub kind: InferenceFailureKind,
    pub code: &'static str,
    /// For 429 responses, the server-advised delay (ms) before retrying, per
    /// the `Retry-After` header. Capped to a sane maximum by the caller.
    pub retry_after_ms: Option<u64>,
}

impl InferenceFailure {
    pub fn transient(code: &'static str) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind: InferenceFailureKind::Transient,
            code,
            retry_after_ms: None,
        })
    }

    /// Transient failure carrying a server-advised retry delay (e.g. from
    /// a 429 `Retry-After` header).
    #[cfg(test)]
    pub(crate) fn transient_with_retry_after(
        code: &'static str,
        retry_after_ms: Option<u64>,
    ) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind: InferenceFailureKind::Transient,
            code,
            retry_after_ms,
        })
    }

    pub fn terminal(code: &'static str) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind: InferenceFailureKind::Terminal,
            code,
            retry_after_ms: None,
        })
    }

    pub fn context_overflow() -> anyhow::Error {
        anyhow::Error::new(Self {
            kind: InferenceFailureKind::ContextOverflow,
            code: "context_overflow",
            retry_after_ms: None,
        })
    }
}
