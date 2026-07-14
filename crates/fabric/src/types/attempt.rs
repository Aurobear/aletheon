//! Durable runtime-attempt contracts shared by the Executive and runtimes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier used to resolve a configured sub-agent runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeId(pub String);

/// Globally unique identifier for one runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptId(pub Uuid);

impl AttemptId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AttemptId {
    fn default() -> Self {
        Self::new()
    }
}

/// Cognitive responsibility assigned to a runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveRole {
    Worker,
    Reviewer,
    Debugger,
    Verifier,
}

/// Stable failure taxonomy used by retry and escalation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Compilation,
    TestFailure,
    PermissionDenied,
    Timeout,
    MissingDependency,
    InvalidAssumption,
    ArchitectureViolation,
    ToolFailure,
    ContextInsufficient,
    ProviderTransient,
    ProviderPermanent,
    Cancelled,
    RepeatedFailure,
}

/// Metered resource use for one attempt.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttemptUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub elapsed_ms: u64,
}

/// Durable state of an attempt record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Bounded evidence emitted by a runtime or verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptEvidence {
    pub kind: String,
    pub summary: String,
    pub content: String,
}

/// Structured successful runtime result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeResult {
    pub output: String,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
}

/// Structured failed runtime result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeFailure {
    pub class: FailureClass,
    pub message: String,
    pub retryable: bool,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
}

impl RuntimeResult {
    /// Return a redacted, byte-bounded copy suitable for durable storage.
    pub fn bounded_for_persistence(&self, max_bytes: usize) -> Self {
        Self {
            output: redact_and_bound(&self.output, max_bytes),
            usage: self.usage.clone(),
            evidence: self
                .evidence
                .iter()
                .map(|item| item.bounded_for_persistence(max_bytes))
                .collect(),
        }
    }
}

impl RuntimeFailure {
    /// Return a redacted, byte-bounded copy suitable for durable storage.
    pub fn bounded_for_persistence(&self, max_bytes: usize) -> Self {
        Self {
            class: self.class,
            message: redact_and_bound(&self.message, max_bytes),
            retryable: self.retryable,
            usage: self.usage.clone(),
            evidence: self
                .evidence
                .iter()
                .map(|item| item.bounded_for_persistence(max_bytes))
                .collect(),
        }
    }
}

impl AttemptEvidence {
    fn bounded_for_persistence(&self, max_bytes: usize) -> Self {
        Self {
            kind: redact_and_bound(&self.kind, max_bytes),
            summary: redact_and_bound(&self.summary, max_bytes),
            content: redact_and_bound(&self.content, max_bytes),
        }
    }
}

fn redact_and_bound(value: &str, max_bytes: usize) -> String {
    let mut words = value.split_whitespace().peekable();
    let mut redacted = String::with_capacity(value.len().min(max_bytes));
    while let Some(word) = words.next() {
        if !redacted.is_empty() {
            redacted.push(' ');
        }
        if word.eq_ignore_ascii_case("bearer") {
            redacted.push_str("[REDACTED]");
            let _ = words.next();
        } else if word.to_ascii_lowercase().starts_with("sk-")
            || credential_assignment(word).is_some()
        {
            redacted.push_str("[REDACTED]");
        } else {
            redacted.push_str(word);
        }
    }

    if redacted.len() <= max_bytes {
        return redacted;
    }
    let mut end = max_bytes;
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    redacted.truncate(end);
    redacted
}

fn credential_assignment(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    matches!(
        key.to_ascii_lowercase().as_str(),
        "token" | "password" | "secret"
    )
    .then_some((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_contracts_round_trip_through_json() {
        let attempt_id = AttemptId::new();
        let encoded = serde_json::to_string(&attempt_id).unwrap();
        assert_eq!(
            serde_json::from_str::<AttemptId>(&encoded).unwrap(),
            attempt_id
        );

        let result = RuntimeResult {
            output: "done".into(),
            usage: AttemptUsage {
                input_tokens: 11,
                output_tokens: 7,
                cost_usd: Some(0.01),
                elapsed_ms: 42,
            },
            evidence: vec![AttemptEvidence {
                kind: "test".into(),
                summary: "suite passed".into(),
                content: "12 passed".into(),
            }],
        };
        let encoded = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<RuntimeResult>(&encoded).unwrap(),
            result
        );
    }

    #[test]
    fn enums_use_stable_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_string(&CognitiveRole::Reviewer).unwrap(),
            "\"reviewer\""
        );
        assert_eq!(
            serde_json::to_string(&FailureClass::ProviderTransient).unwrap(),
            "\"provider_transient\""
        );
        assert_eq!(
            serde_json::to_string(&AttemptStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
    }

    #[test]
    fn persistence_copy_redacts_credentials_and_respects_utf8_boundary() {
        let failure = RuntimeFailure {
            class: FailureClass::ProviderPermanent,
            message: "Bearer abc sk-secret token=value 世界".into(),
            retryable: false,
            usage: AttemptUsage::default(),
            evidence: vec![],
        };

        let stored = failure.bounded_for_persistence(32);
        assert!(!stored.message.contains("abc"));
        assert!(!stored.message.contains("sk-secret"));
        assert!(!stored.message.contains("value"));
        assert!(stored.message.len() <= 32);
        assert!(stored.message.is_char_boundary(stored.message.len()));
    }
}
