use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aletheon_kernel::chronos::Timer;
use async_trait::async_trait;
use tokio::sync::mpsc;

use fabric::{wall_to_datetime, Clock};

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Watches filesystem changes via inotify.
pub struct InotifySource {
    watch_paths: Vec<PathBuf>,
    rx: mpsc::Receiver<PerceptionEvent>,
    tx: mpsc::Sender<PerceptionEvent>,
    #[allow(dead_code)]
    event_id_counter: u64,
    clock: Arc<dyn Clock>,
}

impl InotifySource {
    pub fn new(watch_paths: Vec<PathBuf>, clock: Arc<dyn Clock>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            watch_paths,
            rx,
            tx,
            event_id_counter: 0,
            clock,
        }
    }

    #[allow(dead_code)]
    fn next_id(&mut self) -> u64 {
        self.event_id_counter += 1;
        self.event_id_counter
    }

    /// Start the inotify watcher task.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        for path in &self.watch_paths {
            if !path.exists() {
                tracing::warn!(path = %path.display(), "Watch path does not exist, skipping");
                continue;
            }

            let tx = self.tx.clone();
            let watch_path = path.clone();
            let start_id = self.event_id_counter;
            let clock = self.clock.clone();

            tokio::spawn(async move {
                // Use a simple polling approach for now (inotify crate integration later)
                let mut last_modified = std::collections::HashMap::new();

                loop {
                    Timer::sleep(&*clock, Duration::from_secs(2)).await;

                    if let Ok(entries) = tokio::fs::read_dir(&watch_path).await {
                        let mut entries = entries;
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            if let Ok(metadata) = entry.metadata().await {
                                let modified = metadata
                                    .modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);

                                let path_str = entry.path().to_string_lossy().to_string();
                                let prev = last_modified.insert(path_str.clone(), modified);

                                if prev.is_none() {
                                    let _ = tx
                                        .send(PerceptionEvent {
                                            id: start_id + last_modified.len() as u64,
                                            timestamp: wall_to_datetime(clock.wall_now()),
                                            source: EventSource::Inotify,
                                            category: EventCategory::File,
                                            priority: Priority::Low,
                                            data: EventData::FileCreated { path: path_str },
                                        })
                                        .await;
                                } else if prev != Some(modified) {
                                    let _ = tx
                                        .send(PerceptionEvent {
                                            id: start_id + last_modified.len() as u64 + 1000,
                                            timestamp: wall_to_datetime(clock.wall_now()),
                                            source: EventSource::Inotify,
                                            category: EventCategory::File,
                                            priority: Priority::Low,
                                            data: EventData::FileModified { path: path_str },
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                }
            });
        }

        Ok(())
    }
}

#[async_trait]
impl PerceptionSource for InotifySource {
    fn name(&self) -> &str {
        "inotify"
    }

    async fn poll(&mut self) -> anyhow::Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            events.push(event);
        }
        Ok(events)
    }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux")
    }
}
