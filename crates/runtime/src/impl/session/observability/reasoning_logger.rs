use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Maximum log file size before rotation (100 MB).
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Number of days to retain old log files.
const RETENTION_DAYS: i64 = 7;

/// A single reasoning log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningEntry {
    pub timestamp: String,
    pub session_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// ReasoningLogger: async JSONL logger with rotation and retention.
pub struct ReasoningLogger {
    tx: mpsc::Sender<ReasoningEntry>,
    _handle: tokio::task::JoinHandle<()>,
}

impl ReasoningLogger {
    /// Create a new logger writing to the given directory.
    pub async fn create(session_id: impl Into<String>, log_dir: &Path) -> Result<Self> {
        let session_id = session_id.into();
        fs::create_dir_all(log_dir).await?;

        let log_path = log_dir.join(format!("reasoning-{}.jsonl", session_id));
        let log_dir_owned = log_dir.to_path_buf();

        let (tx, mut rx) = mpsc::channel::<ReasoningEntry>(512);

        let handle = tokio::spawn(async move {
            let current_path = log_path.clone();
            while let Some(entry) = rx.recv().await {
                // Rotate if file exceeds limit
                if let Ok(meta) = fs::metadata(&current_path).await {
                    if meta.len() >= MAX_FILE_SIZE {
                        let rotated = rotate_path(&current_path);
                        if fs::rename(&current_path, &rotated).await.is_ok() {
                            info!(from = %current_path.display(), to = %rotated.display(), "Rotated reasoning log");
                            // Fire-and-forget retention cleanup
                            let dir = log_dir_owned.clone();
                            tokio::spawn(async move {
                                cleanup_old_logs(&dir).await;
                            });
                        }
                    }
                }

                match OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&current_path)
                    .await
                {
                    Ok(mut file) => {
                        let json = serde_json::to_string(&entry).unwrap_or_default();
                        let _ = file.write_all(json.as_bytes()).await;
                        let _ = file.write_all(b"\n").await;
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to open reasoning log file");
                    }
                }

                debug!(kind = %entry.kind, "Reasoning entry logged");
            }
        });

        Ok(Self {
            tx,
            _handle: handle,
        })
    }

    /// Log a reasoning entry.
    pub async fn log(&self, kind: impl Into<String>, payload: serde_json::Value) -> Result<()> {
        let entry = ReasoningEntry {
            timestamp: Utc::now().to_rfc3339(),
            session_id: String::new(), // caller context, can be enriched later
            kind: kind.into(),
            payload,
        };
        self.tx
            .send(entry)
            .await
            .context("ReasoningLogger channel closed")?;
        Ok(())
    }
}

/// Generate a rotated file path by appending a timestamp.
fn rotate_path(original: &Path) -> PathBuf {
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jsonl");
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("reasoning");
    original
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{}_{}.{}", stem, ts, ext))
}

/// Delete log files older than RETENTION_DAYS.
async fn cleanup_old_logs(dir: &Path) {
    let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
    let mut entries = match fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.ends_with(".jsonl") {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            if let Ok(modified) = meta.modified() {
                let modified_dt: chrono::DateTime<Utc> = modified.into();
                if modified_dt < cutoff {
                    if fs::remove_file(entry.path()).await.is_ok() {
                        warn!(file = %name_str, "Removed expired reasoning log");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_and_log() {
        let tmp = TempDir::new().unwrap();
        let logger = ReasoningLogger::create("test-session", tmp.path())
            .await
            .unwrap();

        logger
            .log(
                "thinking",
                serde_json::json!({"thought": "planning next step"}),
            )
            .await
            .unwrap();

        // Give the writer task a moment
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Verify the file exists and has content
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("reasoning"))
            .collect();
        assert!(!entries.is_empty(), "Expected reasoning log file to exist");
    }

    #[test]
    fn test_rotate_path() {
        let p = PathBuf::from("/tmp/reasoning-abc.jsonl");
        let rotated = rotate_path(&p);
        assert!(rotated.to_string_lossy().contains("reasoning-abc_"));
        assert!(rotated.to_string_lossy().ends_with(".jsonl"));
    }
}
