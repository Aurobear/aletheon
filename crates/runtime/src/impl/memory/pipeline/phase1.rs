use std::path::Path;
use std::sync::Arc;

use tokio::fs;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

use super::state_db::StateDatabase;
use super::Phase1Config;

/// Result of a successful Phase 1 extraction for a single session.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub session_id: String,
    pub raw_memory: String,
    pub rollout_summary: String,
    pub rollout_slug: String,
}

/// Phase 1 extractor: pulls session rollouts and extracts memory-relevant content.
///
/// Operates by claiming eligible sessions from the state DB, loading their
/// conversation data, filtering for memory-relevant items, redacting secrets,
/// and truncating to fit within 70% of the context window.
pub struct Phase1Extractor {
    config: Phase1Config,
}

impl Phase1Extractor {
    pub fn new(config: Phase1Config) -> Self {
        Self { config }
    }

    /// Run Phase 1 extraction on eligible sessions.
    ///
    /// Returns the number of sessions successfully processed.
    pub async fn run(
        &self,
        state_db: &mut StateDatabase,
        memory_root: &Path,
    ) -> anyhow::Result<usize> {
        let claim_id = Uuid::new_v4().to_string();
        let now = current_timestamp();

        let max_age_secs = self.config.max_age_days as u64 * 86400;
        let min_idle_secs = self.config.min_idle_hours as u64 * 3600;

        let session_ids = state_db.claim_sessions(
            self.config.max_claims_per_startup,
            now,
            max_age_secs,
            min_idle_secs,
            &claim_id,
        );

        if session_ids.is_empty() {
            info!("No eligible sessions for Phase 1");
            return Ok(0);
        }

        info!(count = session_ids.len(), "Claimed sessions for Phase 1");

        let semaphore = Arc::new(Semaphore::new(self.config.concurrency_limit));
        let mut handles = Vec::new();

        for session_id in session_ids {
            let sem = semaphore.clone();
            let raw_dir = memory_root.join("sessions").join(&session_id);

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await?;
                let result = extract_session(&session_id, &raw_dir).await;
                Ok::<_, anyhow::Error>((session_id, result))
            }));
        }

        let mut succeeded = 0usize;
        for handle in handles {
            match handle.await? {
                Ok((session_id, Ok(result))) => {
                    state_db.mark_succeeded(
                        &session_id,
                        result.raw_memory,
                        result.rollout_summary,
                        result.rollout_slug,
                    )?;
                    succeeded += 1;
                }
                Ok((session_id, Err(e))) => {
                    warn!(session_id, error = %e, "Phase 1 extraction failed");
                    state_db.mark_failed(&session_id, e.to_string())?;
                }
                Err(e) => {
                    warn!(error = %e, "Task join error in Phase 1");
                }
            }
        }

        info!(succeeded, "Phase 1 extraction complete");
        Ok(succeeded)
    }

    /// Get the configured model name.
    pub fn model(&self) -> &str {
        &self.config.model
    }
}

/// Extract memory content from a single session directory.
///
/// Loads the session rollout file, filters for memory-relevant items,
/// redacts potential secrets, and truncates to 70% of the context window.
async fn extract_session(session_id: &str, session_dir: &Path) -> anyhow::Result<ExtractionResult> {
    let rollout_path = session_dir.join("rollout.json");

    if !rollout_path.exists() {
        anyhow::bail!("Session rollout file not found: {}", rollout_path.display());
    }

    let content = fs::read_to_string(&rollout_path).await?;
    let messages: Vec<serde_json::Value> = serde_json::from_str(&content)?;

    // Filter memory-relevant messages (user/assistant turns with substance).
    let memory_items: Vec<String> = messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role")?.as_str()?;
            let text = extract_text_content(msg)?;

            // Skip trivial messages.
            if text.trim().len() < 20 {
                return None;
            }
            // Skip system messages.
            if role == "system" {
                return None;
            }

            Some(format!("[{}]: {}", role, text))
        })
        .collect();

    if memory_items.is_empty() {
        anyhow::bail!("No memory-relevant items found in session {}", session_id);
    }

    // Redact secrets from the combined output.
    let combined = memory_items.join("\n");
    let redacted = redact_secrets(&combined);

    // Truncate to 70% of a typical context window (128k tokens ~= 512k chars).
    let max_chars = (512_000.0 * 0.70) as usize;
    let truncated = if redacted.len() > max_chars {
        &redacted[redacted.len() - max_chars..]
    } else {
        &redacted
    };

    let slug = generate_slug(session_id);
    let summary = format!(
        "Session {} contained {} memory-relevant exchanges",
        session_id,
        memory_items.len()
    );

    Ok(ExtractionResult {
        session_id: session_id.to_string(),
        raw_memory: truncated.to_string(),
        rollout_summary: summary,
        rollout_slug: slug,
    })
}

/// Extract text content from a message JSON object.
fn extract_text_content(msg: &serde_json::Value) -> Option<String> {
    let content = msg.get("content")?;

    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }

    if let Some(arr) = content.as_array() {
        let text: String = arr
            .iter()
            .filter_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            return Some(text);
        }
    }

    None
}

/// Redact potential secrets from text.
///
/// Covers common patterns: API keys, tokens, passwords, private keys.
fn redact_secrets(text: &str) -> String {
    let patterns: &[(&str, &str)] = &[
        (
            r"(?i)(api[_-]?key|token|secret|password|passwd|auth)[=:]\s*\S+",
            "$1=[REDACTED]",
        ),
        (
            r"(?i)-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(RSA\s+)?PRIVATE\s+KEY-----",
            "[REDACTED_PRIVATE_KEY]",
        ),
        (r"(?i)Bearer\s+[A-Za-z0-9\-._~+/]+=*", "Bearer [REDACTED]"),
        (r"ghp_[A-Za-z0-9]{36}", "[REDACTED_GITHUB_TOKEN]"),
        (r"sk-[A-Za-z0-9]{20,}", "[REDACTED_API_KEY]"),
    ];

    let mut result = text.to_string();
    for (pattern, replacement) in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            result = re.replace_all(&result, *replacement).to_string();
        }
    }
    result
}

/// Generate a filesystem-friendly slug for a session.
fn generate_slug(session_id: &str) -> String {
    session_id
        .chars()
        .take(32)
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::memory::pipeline::state_db::{SessionRecord, Stage1Status};
    use tempfile::TempDir;

    fn default_phase1_config() -> Phase1Config {
        Phase1Config {
            concurrency_limit: 4,
            max_claims_per_startup: 50,
            max_age_days: 30,
            min_idle_hours: 2,
            lease_seconds: 3600,
            model: "test-model".to_string(),
        }
    }

    fn make_rollout_json(messages: Vec<(&str, &str)>) -> String {
        let items: Vec<serde_json::Value> = messages
            .iter()
            .map(|(role, text)| {
                serde_json::json!({
                    "role": role,
                    "content": text
                })
            })
            .collect();
        serde_json::to_string(&items).unwrap()
    }

    #[test]
    fn test_redact_secrets() {
        let text = "api_key=sk-abc123defghijklmnop Bearer eyJhbGciOiJIUzI1NiJ9.token ghp_123456789012345678901234567890123456";
        let result = redact_secrets(text);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("sk-abc123"));
        assert!(!result.contains("ghp_123456789012345678901234567890123456"));
    }

    #[test]
    fn test_generate_slug() {
        assert_eq!(generate_slug("abc-123"), "abc-123");
        assert_eq!(generate_slug("hello world!@#"), "hello-world---");
        let long_id = "a".repeat(50);
        assert_eq!(generate_slug(&long_id).len(), 32);
    }

    #[test]
    fn test_extract_text_content_string() {
        let msg = serde_json::json!({"role": "user", "content": "Hello world"});
        assert_eq!(extract_text_content(&msg), Some("Hello world".to_string()));
    }

    #[test]
    fn test_extract_text_content_blocks() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "First part"},
                {"type": "text", "text": "Second part"}
            ]
        });
        assert_eq!(
            extract_text_content(&msg),
            Some("First part Second part".to_string())
        );
    }

    #[tokio::test]
    async fn test_extract_session_success() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("test-session");
        fs::create_dir_all(&session_dir).await.unwrap();

        let rollout = make_rollout_json(vec![
            ("user", "Can you help me write a Rust function that parses CSV files?"),
            ("assistant", "Sure! Here's a function using the csv crate that handles quoted fields and different delimiters."),
            ("user", "Great, now add error handling for malformed rows."),
            ("assistant", "I've added proper error handling with a custom CsvParseError enum."),
        ]);
        fs::write(session_dir.join("rollout.json"), rollout)
            .await
            .unwrap();

        let result = extract_session("test-session", &session_dir).await.unwrap();
        assert_eq!(result.session_id, "test-session");
        assert!(result.raw_memory.contains("[user]"));
        assert!(result.raw_memory.contains("[assistant]"));
        assert!(result.rollout_summary.contains("4"));
    }

    #[tokio::test]
    async fn test_extract_session_missing_file() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("missing-session");
        fs::create_dir_all(&session_dir).await.unwrap();

        let result = extract_session("missing", &session_dir).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_session_empty_messages() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("empty-session");
        fs::create_dir_all(&session_dir).await.unwrap();

        let rollout = make_rollout_json(vec![("user", "hi"), ("assistant", "ok")]);
        fs::write(session_dir.join("rollout.json"), rollout)
            .await
            .unwrap();

        let result = extract_session("empty", &session_dir).await;
        assert!(result.is_err()); // Too short, no memory-relevant items.
    }

    #[tokio::test]
    async fn test_phase1_run_integration() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();
        let sessions_dir = memory_root.join("sessions");

        // Create two sessions with rollout files.
        for sid in &["sess-a", "sess-b"] {
            let dir = sessions_dir.join(sid);
            fs::create_dir_all(&dir).await.unwrap();
            let rollout = make_rollout_json(vec![
                ("user", "Please analyze the performance of this system and suggest improvements."),
                ("assistant", "Based on the metrics, I recommend increasing the buffer size and enabling connection pooling for better throughput."),
            ]);
            fs::write(dir.join("rollout.json"), rollout).await.unwrap();
        }

        let mut db = StateDatabase::new();
        let now = current_timestamp();
        for sid in &["sess-a", "sess-b"] {
            let mut r = SessionRecord::new(
                sid.to_string(),
                sessions_dir.join(sid),
                now - 7200, // created 2h ago
            );
            r.last_used = now - 7200; // idle for 2h
            db.upsert_session(r);
        }

        let config = default_phase1_config();
        let extractor = Phase1Extractor::new(config);
        let count = extractor.run(&mut db, memory_root).await.unwrap();

        assert_eq!(count, 2);

        // Both should be succeeded.
        for sid in &["sess-a", "sess-b"] {
            let rec = db.get_session(sid).unwrap();
            assert_eq!(rec.stage1_status, Stage1Status::Succeeded);
            assert!(rec.raw_memory.is_some());
            assert!(rec.summary.is_some());
            assert!(rec.slug.is_some());
        }
    }
}
