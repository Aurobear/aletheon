use chrono::{DateTime, Utc};
use aletheon_abi::{EventType, Priority};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub source: String,
    pub priority: Priority,
    pub summary: String,
}

pub struct EventLog {
    entries: VecDeque<LogEntry>,
    max_entries: usize,
}

impl EventLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries.min(10000)),
            max_entries,
        }
    }

    pub fn record(&mut self, event: &dyn aletheon_abi::Event) {
        let entry = LogEntry {
            timestamp: Utc::now(),
            event_type: event.event_type(),
            source: event.source().to_string(),
            priority: event.priority(),
            summary: event.summary(),
        };
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn recent(&self, n: usize) -> Vec<&LogEntry> {
        self.entries.iter().rev().take(n).collect()
    }

    pub fn drain(&mut self) -> Vec<LogEntry> {
        self.entries.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct TestEvent {
        event_type: EventType,
        priority: Priority,
        source: String,
    }

    impl aletheon_abi::Event for TestEvent {
        fn event_type(&self) -> EventType { self.event_type.clone() }
        fn priority(&self) -> Priority { self.priority }
        fn source(&self) -> &str { &self.source }
        fn payload(&self) -> &dyn Any { &() }
    }

    fn make_event(event_type: EventType, source: &str) -> TestEvent {
        TestEvent { event_type, priority: Priority::Normal, source: source.to_string() }
    }

    #[test]
    fn test_record_and_len() {
        let mut log = EventLog::new(100);
        assert_eq!(log.len(), 0);
        log.record(&make_event(EventType::UserIntent, "test"));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut log = EventLog::new(3);
        log.record(&make_event(EventType::UserIntent, "1"));
        log.record(&make_event(EventType::ToolError, "2"));
        log.record(&make_event(EventType::ActionCompleted, "3"));
        assert_eq!(log.len(), 3);
        log.record(&make_event(EventType::MemoryStored, "4"));
        assert_eq!(log.len(), 3);
        // Oldest should be evicted
        let recent = log.recent(3);
        assert_eq!(recent[0].source, "4");
        assert_eq!(recent[2].source, "2");
    }

    #[test]
    fn test_recent_returns_reverse_chronological() {
        let mut log = EventLog::new(100);
        log.record(&make_event(EventType::UserIntent, "first"));
        log.record(&make_event(EventType::ToolError, "second"));
        log.record(&make_event(EventType::ActionCompleted, "third"));

        let recent = log.recent(2);
        assert_eq!(recent[0].source, "third");
        assert_eq!(recent[1].source, "second");
    }

    #[test]
    fn test_drain_clears() {
        let mut log = EventLog::new(100);
        log.record(&make_event(EventType::UserIntent, "test"));
        let entries = log.drain();
        assert_eq!(entries.len(), 1);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn test_log_entry_fields() {
        let mut log = EventLog::new(100);
        log.record(&make_event(EventType::BoundaryCheck, "self_field"));
        let entry = log.recent(1)[0];
        assert_eq!(entry.event_type, EventType::BoundaryCheck);
        assert_eq!(entry.source, "self_field");
        assert_eq!(entry.priority, Priority::Normal);
    }
}
