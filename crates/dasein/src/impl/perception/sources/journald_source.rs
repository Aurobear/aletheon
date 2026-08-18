use ::contracts::Clock;
use async_trait::async_trait;
use std::sync::Arc;

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Reads system journal (journald) for important log entries.
pub struct JournaldSource {
    stream: Option<platform::JournalLineStream>,
    event_id_counter: u64,
    min_priority: u8, // 0=emerg .. 7=debug, lower = more important
    clock: Arc<dyn Clock>,
}

impl JournaldSource {
    pub fn new(min_priority: u8, clock: Arc<dyn Clock>) -> Self {
        Self {
            stream: None,
            event_id_counter: 0,
            min_priority,
            clock,
        }
    }

    /// Start the journal reader task.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        self.stream = Some(platform::JournalLineStream::start().await?);
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
        let Some(stream) = self.stream.as_mut() else {
            return Ok(events);
        };
        while let Some(line) = stream.try_recv() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let priority = entry["PRIORITY"]
                .as_str()
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(6);
            if priority > self.min_priority {
                continue;
            }
            let message = entry["MESSAGE"].as_str().unwrap_or("").to_string();
            if message.is_empty() {
                continue;
            }
            self.event_id_counter = self.event_id_counter.saturating_add(1);
            events.push(PerceptionEvent {
                id: self.event_id_counter,
                timestamp: self.clock.wall_now(),
                source: EventSource::Journald,
                category: EventCategory::Service,
                priority: match priority {
                    0..=2 => Priority::Critical,
                    3..=4 => Priority::High,
                    5 => Priority::Normal,
                    _ => Priority::Low,
                },
                data: EventData::JournalEntry {
                    unit: entry["_SYSTEMD_UNIT"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                    message,
                    priority,
                },
            });
        }
        Ok(events)
    }

    fn is_available(&self) -> bool {
        platform::JournalLineStream::is_available()
    }
}
