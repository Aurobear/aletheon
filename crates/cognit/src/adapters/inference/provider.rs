//! Re-exports of the canonical LLM provider trait and types.
//!
//! These items now live in `fabric` (RFC-018 Phase 4, resolves D4) since they
//! are a shared client abstraction, not cognit-specific implementation. This
//! shim keeps every existing cognit-internal path
//! (`crate::r#impl::llm::provider::LlmProvider`, `cognit::r#impl::llm::LlmProvider`,
//! `cognit::llm::provider::LlmProvider`, etc.) resolving unchanged.
pub use fabric::{InferenceCapabilities, LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk, Usage};

/// Tool definition sent to the LLM.
pub use fabric::ToolDefinition;

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
}

impl InferenceFailure {
    pub fn transient(code: &'static str) -> anyhow::Error {
        anyhow::Error::new(Self { kind: InferenceFailureKind::Transient, code })
    }

    pub fn terminal(code: &'static str) -> anyhow::Error {
        anyhow::Error::new(Self { kind: InferenceFailureKind::Terminal, code })
    }

    pub fn context_overflow() -> anyhow::Error {
        anyhow::Error::new(Self { kind: InferenceFailureKind::ContextOverflow, code: "context_overflow" })
    }

    pub fn from_http_status(status: reqwest::StatusCode) -> anyhow::Error {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Self::transient("provider_unavailable")
        } else {
            Self::terminal("provider_rejected_request")
        }
    }
}
