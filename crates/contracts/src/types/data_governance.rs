//! Trust classification and redaction contracts shared by runtime boundaries.

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
                .map(|value| scrub_json_for_projection(value, trust))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let secret_key = matches!(
                        key.to_ascii_lowercase().as_str(),
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
                        key.clone(),
                        if secret_key {
                            serde_json::Value::String("[REDACTED]".into())
                        } else {
                            scrub_json_for_projection(value, trust)
                        },
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}
