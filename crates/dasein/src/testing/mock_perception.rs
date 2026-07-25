use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::r#impl::perception::event::PerceptionEvent;
use crate::r#impl::perception::sources::PerceptionSource;

/// Mock perception source with canned events.
///
/// Events are returned in FIFO order from `poll()`. When the queue is empty,
/// `poll()` returns an empty vec (not an error).
pub struct MockPerceptionSource {
    name: String,
    available: bool,
    events: Mutex<VecDeque<PerceptionEvent>>,
    /// Log of how many times `poll()` was called.
    pub poll_count: Mutex<u32>,
}

impl MockPerceptionSource {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            available: true,
            events: Mutex::new(VecDeque::new()),
            poll_count: Mutex::new(0),
        }
    }

    /// Enqueue a single event to be returned on the next `poll()`.
    pub fn push_event(&self, event: PerceptionEvent) {
        self.events.lock().unwrap().push_back(event);
    }

    /// Enqueue multiple events.
    pub fn push_events(&self, events: impl IntoIterator<Item = PerceptionEvent>) {
        let mut q = self.events.lock().unwrap();
        for e in events {
            q.push_back(e);
        }
    }

    /// Set whether this source reports itself as available.
    pub fn set_available(&mut self, available: bool) {
        self.available = available;
    }

    /// Number of canned events remaining.
    pub fn remaining(&self) -> usize {
        self.events.lock().unwrap().len()
    }
}

#[async_trait]
impl PerceptionSource for MockPerceptionSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn poll(&mut self) -> anyhow::Result<Vec<PerceptionEvent>> {
        *self.poll_count.lock().unwrap() += 1;

        // Drain all currently queued events in one batch
        let mut q = self.events.lock().unwrap();
        let events: Vec<PerceptionEvent> = q.drain(..).collect();
        Ok(events)
    }

    fn is_available(&self) -> bool {
        self.available
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::perception::event::*;
    use fabric::WallTime;

    fn make_test_event(id: u64, msg: &str) -> PerceptionEvent {
        PerceptionEvent {
            id,
            timestamp: WallTime(0),
            source: EventSource::Proc,
            category: EventCategory::Process,
            priority: Priority::Normal,
            data: EventData::Raw {
                message: msg.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_mock_perception_single_poll() {
        let mut source = MockPerceptionSource::new("test");
        source.push_event(make_test_event(1, "event1"));
        source.push_event(make_test_event(2, "event2"));

        let events = source.poll().await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 1);
        assert_eq!(events[1].id, 2);

        // Next poll should return empty
        let events = source.poll().await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_mock_perception_empty_poll() {
        let mut source = MockPerceptionSource::new("test");
        let events = source.poll().await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_mock_perception_poll_count() {
        let mut source = MockPerceptionSource::new("test");
        source.push_event(make_test_event(1, "e1"));

        source.poll().await.unwrap();
        source.poll().await.unwrap();

        assert_eq!(*source.poll_count.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_mock_perception_availability() {
        let mut source = MockPerceptionSource::new("test");
        assert!(source.is_available());

        source.set_available(false);
        assert!(!source.is_available());
    }

    #[tokio::test]
    async fn test_mock_perception_push_events_batch() {
        let source = MockPerceptionSource::new("test");
        let events: Vec<PerceptionEvent> = (0..5)
            .map(|i| make_test_event(i, &format!("event_{i}")))
            .collect();
        source.push_events(events);

        assert_eq!(source.remaining(), 5);
    }
}
