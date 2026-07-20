use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{error, info};

use super::risk_classifier::RiskCategory;
use crate::types::time::WallTime;
use crate::types::tool::PermissionLevel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub audit_id: crate::AuditEventId,
    pub timestamp: WallTime,
    pub session_id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub permission_level: PermissionLevel,
    pub risk_category: RiskCategory,
    pub loop_verdict: String,
    pub result_summary: Option<String>,
    pub is_error: bool,
    pub sandbox_backend: Option<String>,
    pub elapsed_ms: u64,
}

pub struct AuditLogger {
    tx: mpsc::Sender<(
        AuditRecord,
        oneshot::Sender<std::result::Result<(), String>>,
    )>,
}

impl AuditLogger {
    pub fn new(log_path: PathBuf) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<(
            AuditRecord,
            oneshot::Sender<std::result::Result<(), String>>,
        )>(1024);

        // Background writer task
        tokio::spawn(async move {
            use std::io::Write;

            let mut previous_hash = read_last_hash(&log_path).unwrap_or_else(|| "0".repeat(64));

            info!(path = %log_path.display(), "Audit logger started");

            while let Some((record, acknowledgement)) = rx.recv().await {
                let record = sanitize_record(record);
                let outcome = match chained_json(&record, &previous_hash) {
                    Ok(json) => {
                        // Append to JSONL file
                        let mut options = OpenOptions::new();
                        options.create(true).append(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            options.mode(0o600);
                        }
                        match options.open(&log_path) {
                            Ok(mut file) => {
                                if let Err(err) = writeln!(file, "{}", json) {
                                    error!(error = %err, "Failed to write audit record");
                                    Err(format!("failed to write audit record: {err}"))
                                } else if let Some(hash) =
                                    json.get("_record_hash").and_then(serde_json::Value::as_str)
                                {
                                    previous_hash = hash.to_owned();
                                    Ok(())
                                } else {
                                    Err("audit record hash missing after serialization".into())
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to open audit log file");
                                Err(format!("failed to open audit log file: {e}"))
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to serialize audit record");
                        Err(format!("failed to serialize audit record: {e}"))
                    }
                };
                let _ = acknowledgement.send(outcome);
            }

            info!("Audit logger stopped");
        });

        Ok(Self { tx })
    }

    pub async fn log(&self, record: AuditRecord) -> Result<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send((record, ack_tx))
            .await
            .map_err(|e| anyhow::anyhow!("Audit logger channel closed: {}", e))?;
        ack_rx
            .await
            .map_err(|e| anyhow::anyhow!("Audit writer dropped acknowledgement: {e}"))?
            .map_err(anyhow::Error::msg)
    }

    pub fn log_sync(&self, record: AuditRecord) {
        let (ack_tx, _ack_rx) = oneshot::channel();
        let _ = self.tx.try_send((record, ack_tx));
    }
}

const MAX_AUDIT_STRING_BYTES: usize = 8 * 1024;

fn sanitize_record(mut record: AuditRecord) -> AuditRecord {
    record.session_id = redact_sensitive_text(&record.session_id, 256);
    record.turn_id = redact_sensitive_text(&record.turn_id, 256);
    record.tool_name = redact_sensitive_text(&record.tool_name, 256);
    record.loop_verdict = redact_sensitive_text(&record.loop_verdict, 512);
    record.sandbox_backend = record
        .sandbox_backend
        .map(|value| redact_sensitive_text(&value, 256));
    record.result_summary = record
        .result_summary
        .map(|value| redact_sensitive_text(&value, MAX_AUDIT_STRING_BYTES));
    redact_json(&mut record.args, None);
    record
}

fn chained_json(
    record: &AuditRecord,
    previous_hash: &str,
) -> serde_json::Result<serde_json::Value> {
    let record_value = serde_json::to_value(record)?;
    let canonical = serde_json::to_vec(&record_value)?;
    let mut hasher = Sha256::new();
    hasher.update(previous_hash.as_bytes());
    hasher.update(&canonical);
    let record_hash = format!("{:x}", hasher.finalize());
    let mut object = record_value.as_object().cloned().unwrap_or_default();
    object.insert("_previous_hash".into(), previous_hash.into());
    object.insert("_record_hash".into(), record_hash.into());
    Ok(object.into())
}

fn read_last_hash(path: &PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let last = content.lines().rev().find(|line| !line.trim().is_empty())?;
    serde_json::from_str::<serde_json::Value>(last)
        .ok()?
        .get("_record_hash")?
        .as_str()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
}

fn redact_json(value: &mut serde_json::Value, key: Option<&str>) {
    if key.is_some_and(sensitive_key) {
        *value = serde_json::Value::String("[REDACTED]".into());
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                redact_json(value, Some(key));
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json(value, key);
            }
        }
        serde_json::Value::String(text) => {
            *text = redact_sensitive_text(text, MAX_AUDIT_STRING_BYTES);
        }
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "token",
        "password",
        "secret",
        "api_key",
        "email_body",
        "provider_payload",
        "prompt",
        "credentials",
    ]
    .iter()
    .any(|needle| key == *needle || key.ends_with(&format!("_{needle}")))
}

/// Redact common credential forms and bound untrusted text before it reaches
/// durable audit or support output.
pub fn redact_sensitive_text(value: &str, max_bytes: usize) -> String {
    let normalized: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let mut words = normalized.split_whitespace().peekable();
    let mut redacted = String::with_capacity(normalized.len().min(max_bytes));
    while let Some(word) = words.next() {
        if !redacted.is_empty() {
            redacted.push(' ');
        }
        let lower = word.to_ascii_lowercase();
        if matches!(lower.as_str(), "bearer" | "authorization:" | "cookie:") {
            redacted.push_str("[REDACTED]");
            let _ = words.next();
        } else if lower.starts_with("sk-")
            || lower.starts_with("ghp_")
            || lower.starts_with("xox")
            || lower.contains("/etc/aletheon/credentials/")
            || assignment_is_sensitive(&lower)
        {
            redacted.push_str("[REDACTED]");
        } else {
            redacted.push_str(word);
        }
    }
    if redacted.len() > max_bytes {
        let mut end = max_bytes;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        redacted.truncate(end);
    }
    redacted
}

fn assignment_is_sensitive(word: &str) -> bool {
    let Some((key, _)) = word.split_once('=') else {
        return false;
    };
    sensitive_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_keys_tokens_control_characters_and_bounds_text() {
        let mut value = serde_json::json!({
            "Authorization": "Bearer canary-auth",
            "email_body": "canary-email",
            "nested": {"message": "token=canary-token\nnext sk-canary"},
        });
        redact_json(&mut value, None);
        let rendered = value.to_string();
        assert!(!rendered.contains("canary"));
        assert!(!rendered.contains("\\n"));
        assert!(redact_sensitive_text(&"x".repeat(20_000), 128).len() <= 128);
    }

    #[test]
    fn audit_hash_chain_links_records() {
        let record = AuditRecord {
            audit_id: crate::AuditEventId::new(),
            timestamp: WallTime(1),
            session_id: "session".into(),
            turn_id: "turn".into(),
            tool_name: "tool".into(),
            args: serde_json::json!({}),
            permission_level: PermissionLevel::L0,
            risk_category: RiskCategory::ReadOnly,
            loop_verdict: "allow".into(),
            result_summary: None,
            is_error: false,
            sandbox_backend: None,
            elapsed_ms: 2,
        };
        let first = chained_json(&record, &"0".repeat(64)).unwrap();
        let first_hash = first["_record_hash"].as_str().unwrap();
        let second = chained_json(&record, first_hash).unwrap();
        assert_eq!(second["_previous_hash"], first_hash);
        assert_ne!(second["_record_hash"], first["_record_hash"]);
    }
}
