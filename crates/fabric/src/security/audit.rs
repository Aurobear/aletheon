use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, info};

use super::risk_classifier::RiskCategory;
use crate::types::time::WallTime;
use crate::types::tool::PermissionLevel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
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
    tx: mpsc::Sender<AuditRecord>,
}

impl AuditLogger {
    pub fn new(log_path: PathBuf) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<AuditRecord>(1024);

        // Background writer task
        tokio::spawn(async move {
            use std::io::Write;

            info!(path = %log_path.display(), "Audit logger started");

            while let Some(record) = rx.recv().await {
                match serde_json::to_string(&record) {
                    Ok(json) => {
                        // Append to JSONL file
                        match std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&log_path)
                        {
                            Ok(mut file) => {
                                if writeln!(file, "{}", json).is_err() {
                                    error!("Failed to write audit record");
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to open audit log file");
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to serialize audit record");
                    }
                }
            }

            info!("Audit logger stopped");
        });

        Ok(Self { tx })
    }

    pub async fn log(&self, record: AuditRecord) -> Result<()> {
        self.tx
            .send(record)
            .await
            .map_err(|e| anyhow::anyhow!("Audit logger channel closed: {}", e))
    }

    pub fn log_sync(&self, record: AuditRecord) {
        let _ = self.tx.try_send(record);
    }
}
