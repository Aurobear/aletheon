//! Re-exports of the canonical LLM provider trait and types.
//!
//! These items now live in `fabric` (RFC-018 Phase 4, resolves D4) since they
//! are a shared client abstraction, not cognit-specific implementation. This
//! Cognit uses the shared contract internally and exposes it through the stable
//! `cognit::inference::provider` facade. Provider transports stay private.
pub use fabric::{
    InferenceCapabilities, LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk,
    Usage,
};

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

    /// Cap on the `Retry-After` delay we will honor, to avoid unbounded waits.
    const MAX_RETRY_AFTER_MS: u64 = 60_000;

    /// Parse a `Retry-After` header value as whole seconds (HTTP-date form
    /// is not handled; absence or malformed values simply yield `None`).
    fn parse_retry_after_ms(response: &reqwest::Response) -> Option<u64> {
        let value = response.headers().get(reqwest::header::RETRY_AFTER)?;
        let secs: u64 = value.to_str().ok()?.trim().parse().ok()?;
        Some(secs.saturating_mul(1_000).min(Self::MAX_RETRY_AFTER_MS))
    }

    pub fn from_http_status(response: &reqwest::Response) -> anyhow::Error {
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_ms = Self::parse_retry_after_ms(response);
            Self::transient_with_retry_after("provider_unavailable", retry_after_ms)
        } else if status.is_server_error() {
            Self::transient("provider_unavailable")
        } else {
            Self::terminal("provider_rejected_request")
        }
    }
}
