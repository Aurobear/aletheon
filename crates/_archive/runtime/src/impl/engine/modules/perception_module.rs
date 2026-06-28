//! PerceptionModule — replaces PerceptionBridge mpsc with bus topic publishing.
//!
//! Publishes perception events to topic "perception.events" on the CommunicationBus.
//! Critical/High events are published immediately; others are buffered and flushed
//! periodically or when the buffer reaches capacity.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use base::envelope::*;
use base::envelope::Payload;
use base::CommunicationBus;

use dasein::r#impl::perception::event::{PerceptionEvent, Priority as EventPriority};

use super::PerceptionEventMsg;

const PERCEPTION_TOPIC: &str = "perception.events";
const DEFAULT_BUFFER_MAX: usize = 50;
const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// Bus-based perception event publisher.
///
/// Replaces the old `PerceptionBridge` mpsc pattern. Receives `PerceptionEvent`s
/// from system monitors via an mpsc channel and publishes them to the
/// "perception.events" topic on the CommunicationBus.
pub struct PerceptionModule {
    event_rx: mpsc::Receiver<PerceptionEvent>,
    buffer: Vec<PerceptionEvent>,
    buffer_max: usize,
    flush_interval: Duration,
}

impl PerceptionModule {
    /// Create a new PerceptionModule.
    pub fn new(event_rx: mpsc::Receiver<PerceptionEvent>) -> Self {
        Self {
            event_rx,
            buffer: Vec::new(),
            buffer_max: DEFAULT_BUFFER_MAX,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        }
    }

    /// Set the buffer capacity before auto-flush.
    pub fn with_buffer_max(mut self, max: usize) -> Self {
        self.buffer_max = max;
        self
    }

    /// Set the flush interval.
    pub fn with_flush_interval(mut self, interval: Duration) -> Self {
        self.flush_interval = interval;
        self
    }

    /// Run the module loop: receive events, publish or buffer, flush periodically.
    pub async fn run(mut self, bus: Arc<CommunicationBus>) {
        let mut flush_timer = tokio::time::interval(self.flush_interval);
        let mut event_rx = Some(self.event_rx);
        let mut buffer = std::mem::take(&mut self.buffer);
        let buffer_max = self.buffer_max;
        info!(
            "PerceptionModule: started, publishing to topic '{}'",
            PERCEPTION_TOPIC
        );

        loop {
            tokio::select! {
                event = async {
                    if let Some(ref mut rx) = event_rx {
                        rx.recv().await
                    } else {
                        None
                    }
                } => {
                    match event {
                        Some(e) => {
                            handle_event(&bus, &mut buffer, buffer_max, e).await;
                        }
                        None => {
                            event_rx = None;
                            if buffer.is_empty() {
                                break;
                            }
                        }
                    }
                }
                _ = flush_timer.tick() => {
                    if !buffer.is_empty() {
                        flush_buffer(&bus, &mut buffer).await;
                    }
                    if event_rx.is_none() {
                        break;
                    }
                }
            }
        }

        info!("PerceptionModule: shut down");
    }
}

async fn handle_event(
    bus: &CommunicationBus,
    buffer: &mut Vec<PerceptionEvent>,
    buffer_max: usize,
    event: PerceptionEvent,
) {
    match event.priority {
        EventPriority::Critical | EventPriority::High => {
            debug!(
                priority = ?event.priority,
                summary = %event.summary(),
                "Publishing critical/high perception event immediately"
            );
            publish_event(bus, &event).await;
        }
        _ => {
            buffer.push(event);
            if buffer.len() >= buffer_max {
                flush_buffer(bus, buffer).await;
            }
        }
    }
}

async fn flush_buffer(bus: &CommunicationBus, buffer: &mut Vec<PerceptionEvent>) {
    if buffer.is_empty() {
        return;
    }
    let events: Vec<_> = buffer.drain(..).collect();
    debug!(count = events.len(), "Flushing buffered perception events");
    for event in &events {
        publish_event(bus, event).await;
    }
}

/// Publish a single perception event to the bus topic.
async fn publish_event(bus: &CommunicationBus, event: &PerceptionEvent) {
    let msg = PerceptionEventMsg {
        source: format!("{:?}", event.source),
        priority: format!("{:?}", event.priority),
        summary: event.summary(),
        raw: serde_json::to_value(&event.data).unwrap_or_default(),
    };

    let envelope = Envelope::publish(
        Endpoint::Module(ModuleId::Perception),
        PERCEPTION_TOPIC,
        Payload::Json(serde_json::to_value(&msg).unwrap_or_default()),
    );

    if let Err(e) = bus.publish(envelope).await {
        warn!("PerceptionModule: failed to publish event: {}", e);
    }
}
