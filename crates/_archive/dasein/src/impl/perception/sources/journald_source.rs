use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::mpsc;

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Reads system journal (journald) for important log entries.
pub struct JournaldSource {
    rx: mpsc::Receiver<PerceptionEvent>,
    tx: mpsc::Sender<PerceptionEvent>,
    #[allow(dead_code)]
    event_id_counter: u64,
    min_priority: u8, // 0=emerg .. 7=debug, lower = more important
}

impl JournaldSource {
    pub fn new(min_priority: u8) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            rx,
            tx,
            event_id_counter: 0,
            min_priority,
        }
    }

    /// Start the journal reader task.
    pub async fn start(&self) -> anyhow::Result<()> {
        let tx = self.tx.clone();
        let min_priority = self.min_priority;

        tokio::spawn(async move {
            // Use journalctl --follow to stream journal entries
            let mut child = match tokio::process::Command::new("journalctl")
                .args(["-f", "-o", "json", "--no-pager"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to start journalctl");
                    return;
                }
            };

            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    tracing::error!("Failed to capture journalctl stdout");
                    return;
                }
            };

            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut id = 0u64;

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) {
                    let priority = entry["PRIORITY"]
                        .as_str()
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(6);

                    if priority > min_priority {
                        continue;
                    }

                    let unit = entry["_SYSTEMD_UNIT"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();
                    let message = entry["MESSAGE"].as_str().unwrap_or("").to_string();

                    if message.is_empty() {
                        continue;
                    }

                    id += 1;
                    let _ = tx
                        .send(PerceptionEvent {
                            id,
                            timestamp: Utc::now(),
                            source: EventSource::Journald,
                            category: EventCategory::Service,
                            priority: match priority {
                                0..=2 => Priority::Critical,
                                3..=4 => Priority::High,
                                5 => Priority::Normal,
                                _ => Priority::Low,
                            },
                            data: EventData::JournalEntry {
                                unit,
                                message,
                                priority,
                            },
                        })
                        .await;
                }
            }

            let _ = child.wait().await;
        });

        Ok(())
    }
}

#[async_trait]
impl PerceptionSource for JournaldSource {
    fn name(&self) -> &str {
        "journald"
    }

    async fn poll(&mut self) -> anyhow::Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        Ok(events)
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("journalctl")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }
}
