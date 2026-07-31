//! Shared trust, data-classification, and persistence scrub contract.

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const SCRUB_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentTrust {
    HostTrusted,
    ProviderTrusted,
    ExternalUntrusted,
    ToolUntrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedContent {
    pub content: String,
    pub trust: ContentTrust,
    pub classification: DataClassification,
    pub scrub_policy_version: u32,
    pub redactions: usize,
}

/// Scrub content before it enters a persistence or external projection that
/// does not require the original secret-bearing payload.
pub fn scrub_for_projection(value: &str, trust: ContentTrust) -> GovernedContent {
    let patterns = [
        r"(?i)(api[_-]?key|access[_-]?token|refresh[_-]?token|password|client[_-]?secret)\s*[:=]\s*[^\s,;]+",
        r"(?i)authorization\s*:\s*(bearer|basic)\s+[^\s]+",
        r"-----BEGIN [^-]+ PRIVATE KEY-----[\s\S]*?-----END [^-]+ PRIVATE KEY-----",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\b(?:ghp|github_pat)_[A-Za-z0-9_]{20,}\b",
        r"\bsk-[A-Za-z0-9._-]{8,}\b",
        r"\bkey-[A-Za-z0-9._-]{8,}\b",
        r"(?i)(?:api\s*)?密钥\s*[:=]?\s*[A-Za-z0-9._-]{8,}",
        r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
    ];
    let mut content = value.to_owned();
    let mut redactions = 0;
    for pattern in patterns {
        let regex = Regex::new(pattern).expect("static scrub regex");
        redactions += regex.find_iter(&content).count();
        content = regex.replace_all(&content, "[REDACTED]").into_owned();
    }
    GovernedContent {
        content,
        trust,
        classification: if redactions > 0 {
            DataClassification::Restricted
        } else if matches!(
            trust,
            ContentTrust::ExternalUntrusted | ContentTrust::ToolUntrusted
        ) {
            DataClassification::Internal
        } else {
            DataClassification::Public
        },
        scrub_policy_version: SCRUB_POLICY_VERSION,
        redactions,
    }
}

pub fn scrub_json_for_projection(
    value: &serde_json::Value,
    trust: ContentTrust,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => {
            serde_json::Value::String(scrub_for_projection(value, trust).content)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|v| scrub_json_for_projection(v, trust))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(k, v)| {
                    let secret_key = matches!(
                        k.to_ascii_lowercase().as_str(),
                        "password"
                            | "token"
                            | "access_token"
                            | "refresh_token"
                            | "api_key"
                            | "authorization"
                            | "secret"
                            | "client_secret"
                    );
                    (
                        k.clone(),
                        if secret_key {
                            serde_json::Value::String("[REDACTED]".into())
                        } else {
                            scrub_json_for_projection(v, trust)
                        },
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrubs_secrets_and_pii_with_typed_result() {
        let result = scrub_for_projection(
            "api_key=secret user=a@example.com",
            ContentTrust::ToolUntrusted,
        );
        assert_eq!(result.content, "[REDACTED] user=[REDACTED]");
        assert_eq!(result.classification, DataClassification::Restricted);
        assert_eq!(result.redactions, 2);
    }

    #[test]
    fn recursively_scrubs_structured_secret_fields() {
        let input = serde_json::json!({
            "nested": {"access_token": "secret", "safe": "value"},
            "contact": "a@example.com"
        });
        let scrubbed = scrub_json_for_projection(&input, ContentTrust::ExternalUntrusted);
        assert_eq!(scrubbed["nested"]["access_token"], "[REDACTED]");
        assert_eq!(scrubbed["nested"]["safe"], "value");
        assert_eq!(scrubbed["contact"], "[REDACTED]");
    }

    #[test]
    fn scrubs_unlabelled_provider_keys_and_chinese_key_labels() {
        let result = scrub_for_projection(
            "old sk-exampleSecret123 and key-crossSession456; API 密钥 abcdefgh123456",
            ContentTrust::ExternalUntrusted,
        );
        assert_eq!(result.content, "old [REDACTED] and [REDACTED]; [REDACTED]");
        assert_eq!(result.classification, DataClassification::Restricted);
        assert_eq!(result.redactions, 3);
    }
}
