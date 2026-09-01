//! Re-exports of the canonical LLM provider trait and types.
//!
//! These items now live in `fabric` (RFC-018 Phase 4, resolves D4) since they
//! are a shared client abstraction, not cognit-specific implementation. This
//! Cognit uses the shared contract internally and exposes it through the stable
//! `cognit::inference::provider` facade. Provider transports stay private.
pub use ::contracts::{
    canonicalize_tool_definitions, CacheTelemetry, InferenceUsage, LlmProvider, LlmResponse,
    LlmStream, StopReason, StreamChunk,
};

/// Tool definition sent to the LLM.
pub use ::contracts::ToolDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceFailureKind {
    Transient,
    Terminal,
}

#[derive(Debug)]
pub struct InferenceFailure {
    pub kind: InferenceFailureKind,
    pub code: &'static str,
    pub provider_identity: Option<String>,
    pub http_status: Option<u16>,
    pub body_summary: Option<String>,
    /// For 429 responses, the server-advised delay (ms) before retrying, per
    /// the `Retry-After` header. Capped to a sane maximum by the caller.
    pub retry_after_ms: Option<u64>,
}

impl InferenceFailure {
    pub fn transient(code: &'static str) -> anyhow::Error {
        anyhow::Error::new(Self {
            kind: InferenceFailureKind::Transient,
            code,
            provider_identity: None,
            http_status: None,
            body_summary: None,
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

    pub async fn from_http_response(
        response: reqwest::Response,
        provider_identity: &str,
    ) -> anyhow::Error {
        let status = response.status();
        let retry_after_ms = (status == reqwest::StatusCode::TOO_MANY_REQUESTS)
            .then(|| Self::parse_retry_after_ms(&response))
            .flatten();
        let body_summary = bounded_redacted_body(response).await;
        let failure = Self {
            kind: if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                InferenceFailureKind::Transient
            } else {
                InferenceFailureKind::Terminal
            },
            code: if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                "provider_unavailable"
            } else {
                "provider_rejected_request"
            },
            provider_identity: Some(bounded_identity(provider_identity)),
            http_status: Some(status.as_u16()),
            body_summary,
            retry_after_ms,
        };
        anyhow::Error::new(failure)
    }
}

impl std::fmt::Display for InferenceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.code)?;
        if let Some(provider) = &self.provider_identity {
            write!(formatter, " provider={provider}")?;
        }
        if let Some(status) = self.http_status {
            write!(formatter, " http_status={status}")?;
        }
        Ok(())
    }
}

impl std::error::Error for InferenceFailure {}

const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 4096;
const MAX_PROVIDER_ERROR_SUMMARY_CHARS: usize = 1024;

async fn bounded_redacted_body(response: reqwest::Response) -> Option<String> {
    use futures::StreamExt;

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining || bytes.len() == MAX_PROVIDER_ERROR_BODY_BYTES {
            truncated = true;
            break;
        }
    }
    if bytes.is_empty() {
        return None;
    }
    let raw = String::from_utf8_lossy(&bytes);
    let mut redacted = serde_json::from_str::<serde_json::Value>(&raw).map_or_else(
        |_| redact_text(&raw),
        |mut value| {
            redact_json(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| "<unrenderable>".into())
        },
    );
    redacted = redacted.replace(['\r', '\n'], " ");
    let mut summary = redacted
        .chars()
        .take(MAX_PROVIDER_ERROR_SUMMARY_CHARS)
        .collect::<String>();
    if truncated || redacted.chars().count() > MAX_PROVIDER_ERROR_SUMMARY_CHARS {
        summary.push_str("[truncated]");
    }
    Some(summary)
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String("<redacted>".into());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_json),
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "apikey",
        "authorization",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn redact_text(value: &str) -> String {
    let mut redacted = value.to_owned();
    for marker in [
        "api_key=",
        "api-key:",
        "api key:",
        "authorization: bearer ",
        "secret=",
        "token=",
        "token:",
    ] {
        let mut search_from = 0;
        while let Some(offset) = redacted[search_from..].to_ascii_lowercase().find(marker) {
            let value_start = search_from + offset + marker.len();
            let value_end = redacted[value_start..]
                .find(char::is_whitespace)
                .map(|offset| value_start + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(value_start..value_end, "<redacted>");
            search_from = value_start + "<redacted>".len();
        }
    }
    redacted
}

fn bounded_identity(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .chars()
        .take(128)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failure_diagnostic_keeps_identity_status_and_redacts_sensitive_json() {
        let mut body = serde_json::json!({
            "error": {
                "message": "model missing-model was not found",
                "api_key": "fixture-secret",
                "nested": {"access_token": "fixture-token"}
            }
        });
        redact_json(&mut body);
        let failure = InferenceFailure {
            kind: InferenceFailureKind::Terminal,
            code: "provider_rejected_request",
            provider_identity: Some(bounded_identity("provider-a\nignored")),
            http_status: Some(404),
            body_summary: Some(serde_json::to_string(&body).unwrap()),
            retry_after_ms: None,
        };

        let summary = failure.body_summary.as_deref().unwrap();
        assert!(summary.contains("missing-model"));
        assert!(!summary.contains("fixture-secret"));
        assert!(!summary.contains("fixture-token"));
        assert!(summary.contains("<redacted>"));

        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("provider=provider-a ignored"));
        assert!(diagnostic.contains("http_status=404"));
        assert!(!diagnostic.contains("missing-model"));
        assert!(!diagnostic.contains("fixture-secret"));
        assert!(!diagnostic.contains("fixture-token"));
    }

    #[test]
    fn plaintext_redaction_is_bounded_to_known_secret_markers() {
        let redacted = redact_text(
            "authorization: bearer fixture-auth token=fixture-token safe=model-not-found",
        );
        assert!(!redacted.contains("fixture-auth"));
        assert!(!redacted.contains("fixture-token"));
        assert!(redacted.contains("model-not-found"));
    }
}
