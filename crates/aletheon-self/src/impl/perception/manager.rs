use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::aggregator::EventAggregator;
use super::event::*;
use super::sources::inotify_source::InotifySource;
use super::sources::journald_source::JournaldSource;
use super::sources::proc_source::ProcSource;
use super::sources::PerceptionSource;

/// Unified perception manager — polls all sources and emits aggregated events.
pub struct PerceptionManager {
    proc_source: ProcSource,
    inotify_source: Option<InotifySource>,
    journald_source: Option<JournaldSource>,
    aggregator: EventAggregator,
    event_tx: mpsc::Sender<PerceptionEvent>,
    poll_interval: std::time::Duration,
}

impl PerceptionManager {
    pub fn new(
        event_tx: mpsc::Sender<PerceptionEvent>,
        watch_paths: Vec<PathBuf>,
        enable_journald: bool,
    ) -> Self {
        let inotify = if !watch_paths.is_empty() {
            Some(InotifySource::new(watch_paths))
        } else {
            None
        };

        let journald = if enable_journald {
            Some(JournaldSource::new(4)) // warning and above
        } else {
            None
        };

        Self {
            proc_source: ProcSource::new(),
            inotify_source: inotify,
            journald_source: journald,
            aggregator: EventAggregator::new(),
            event_tx,
            poll_interval: std::time::Duration::from_secs(5),
        }
    }

    /// Start all sources and the polling loop.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        info!("Starting perception manager");

        // Start inotify watcher
        if let Some(ref mut inotify) = self.inotify_source {
            inotify.start().await?;
            info!("Inotify watcher started");
        }

        // Start journald reader
        if let Some(ref journald) = self.journald_source {
            journald.start().await?;
            info!("Journald reader started");
        }

        // Main polling loop
        let mut interval = tokio::time::interval(self.poll_interval);
        loop {
            interval.tick().await;

            let mut all_events = Vec::new();

            // Poll /proc
            match self.proc_source.poll().await {
                Ok(events) => all_events.extend(events),
                Err(e) => warn!(error = %e, "Failed to poll /proc"),
            }

            // Poll inotify
            if let Some(ref mut inotify) = self.inotify_source {
                match inotify.poll().await {
                    Ok(events) => all_events.extend(events),
                    Err(e) => warn!(error = %e, "Failed to poll inotify"),
                }
            }

            // Poll journald
            if let Some(ref mut journald) = self.journald_source {
                match journald.poll().await {
                    Ok(events) => all_events.extend(events),
                    Err(e) => warn!(error = %e, "Failed to poll journald"),
                }
            }

            // Aggregate
            let events = self.aggregator.aggregate(all_events);

            // Send to consumers
            for event in events {
                debug!(
                    source = ?event.source,
                    priority = ?event.priority,
                    summary = %event.summary(),
                    "Perception event"
                );
                if self.event_tx.send(event).await.is_err() {
                    warn!("Event receiver dropped, stopping perception manager");
                    return Ok(());
                }
            }
        }
    }
}
