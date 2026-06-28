use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::event::{PerceptionEvent, Priority};
use base::Message;

/// Bridges perception events into the engine as system messages.
pub struct PerceptionBridge {
    event_rx: mpsc::Receiver<PerceptionEvent>,
    engine_tx: mpsc::Sender<PerceptionInjection>,
    buffer: Vec<PerceptionEvent>,
    buffer_max: usize,
    flush_interval: Duration,
}

/// Injection type for the engine.
#[derive(Debug)]
pub enum PerceptionInjection {
    /// Critical/High priority -- inject immediately
    Immediate(Message),
    /// Medium/Low priority -- batch at turn boundary
    Batch(Vec<PerceptionEvent>),
}

impl PerceptionBridge {
    pub fn new(
        event_rx: mpsc::Receiver<PerceptionEvent>,
        engine_tx: mpsc::Sender<PerceptionInjection>,
    ) -> Self {
        Self {
            event_rx,
            engine_tx,
            buffer: Vec::new(),
            buffer_max: 50,
            flush_interval: Duration::from_secs(30),
        }
    }

    /// Run the bridge loop. Call this in a tokio::spawn.
    pub async fn run(&mut self) {
        let mut flush_timer = tokio::time::interval(self.flush_interval);
        info!("Perception bridge started");

        loop {
            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    self.handle_event(event).await;
                }
                _ = flush_timer.tick() => {
                    if !self.buffer.is_empty() {
                        self.flush_buffer().await;
                    }
                }
                else => {
                    info!("Perception bridge: all senders dropped");
                    break;
                }
            }
        }
    }

    async fn handle_event(&mut self, event: PerceptionEvent) {
        match event.priority {
            Priority::Critical | Priority::High => {
                let msg = event_to_message(&event);
                debug!(summary = %event.summary(), "Injecting critical perception event");
                if self
                    .engine_tx
                    .send(PerceptionInjection::Immediate(msg))
                    .await
                    .is_err()
                {
                    warn!("Engine receiver dropped, buffering event");
                    self.buffer.push(event);
                }
            }
            _ => {
                self.buffer.push(event);
                if self.buffer.len() >= self.buffer_max {
                    self.flush_buffer().await;
                }
            }
        }
    }

    async fn flush_buffer(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let events: Vec<_> = self.buffer.drain(..).collect();
        debug!(count = events.len(), "Flushing buffered perception events");
        let _ = self
            .engine_tx
            .send(PerceptionInjection::Batch(events))
            .await;
    }
}

fn event_to_message(event: &PerceptionEvent) -> Message {
    Message::system(format!(
        "[Perception Alert] source={:?}, priority={:?}, summary={}",
        event.source,
        event.priority,
        event.summary()
    ))
}

#[cfg(test)]
mod tests {
    use super::super::event::*;
    use super::*;
    use base::ContentBlock;
    use chrono::Utc;

    fn make_event(priority: Priority, summary_data: EventData) -> PerceptionEvent {
        PerceptionEvent {
            id: 1,
            timestamp: Utc::now(),
            source: EventSource::Proc,
            category: EventCategory::Process,
            priority,
            data: summary_data,
        }
    }

    #[tokio::test]
    async fn test_critical_event_immediate_injection() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (injection_tx, mut injection_rx) = mpsc::channel(16);

        let mut bridge = PerceptionBridge::new(event_rx, injection_tx);

        // Send a critical event
        let event = make_event(
            Priority::Critical,
            EventData::Raw {
                message: "disk full".to_string(),
            },
        );
        event_tx.send(event).await.unwrap();

        // Handle one event
        let received = bridge.event_rx.recv().await.unwrap();
        bridge.handle_event(received).await;

        // Should get an Immediate injection
        let injection = injection_rx.recv().await.unwrap();
        match injection {
            PerceptionInjection::Immediate(msg) => {
                assert!(msg.content.iter().any(|c| matches!(c, ContentBlock::Text { text } if text.contains("Perception Alert"))));
            }
            _ => panic!("Expected Immediate injection for critical event"),
        }
    }

    #[tokio::test]
    async fn test_low_event_buffered() {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (injection_tx, mut injection_rx) = mpsc::channel(16);

        let mut bridge = PerceptionBridge::new(event_rx, injection_tx);

        // Send a low-priority event
        let event = make_event(
            Priority::Low,
            EventData::Raw {
                message: "routine check".to_string(),
            },
        );
        event_tx.send(event).await.unwrap();

        // Handle one event
        let received = bridge.event_rx.recv().await.unwrap();
        bridge.handle_event(received).await;

        // Should NOT get injection yet (buffered)
        assert!(injection_rx.try_recv().is_err());

        // Buffer should have 1 event
        assert_eq!(bridge.buffer.len(), 1);
    }

    #[tokio::test]
    async fn test_buffer_flush() {
        let (_event_tx, event_rx) = mpsc::channel(16);
        let (injection_tx, mut injection_rx) = mpsc::channel(16);

        let mut bridge = PerceptionBridge::new(event_rx, injection_tx);

        // Manually add events to buffer
        for i in 0..3 {
            bridge.buffer.push(make_event(
                Priority::Low,
                EventData::Raw {
                    message: format!("event {}", i),
                },
            ));
        }

        bridge.flush_buffer().await;

        let injection = injection_rx.recv().await.unwrap();
        match injection {
            PerceptionInjection::Batch(events) => {
                assert_eq!(events.len(), 3);
            }
            _ => panic!("Expected Batch injection for buffered events"),
        }
    }

    #[tokio::test]
    async fn test_buffer_max_flush() {
        let (event_tx, event_rx) = mpsc::channel(100);
        let (injection_tx, mut injection_rx) = mpsc::channel(100);

        let mut bridge = PerceptionBridge::new(event_rx, injection_tx);
        bridge.buffer_max = 5; // small for testing

        // Send 5 low-priority events
        for i in 0..5 {
            let event = make_event(
                Priority::Normal,
                EventData::Raw {
                    message: format!("event {}", i),
                },
            );
            event_tx.send(event).await.unwrap();
        }

        // Handle all 5
        for _ in 0..5 {
            let received = bridge.event_rx.recv().await.unwrap();
            bridge.handle_event(received).await;
        }

        // Should have flushed
        let injection = injection_rx.recv().await.unwrap();
        match injection {
            PerceptionInjection::Batch(events) => {
                assert_eq!(events.len(), 5);
            }
            _ => panic!("Expected Batch injection after buffer_max reached"),
        }
    }
}
