//! # Core Event
//!
//! Re-exports the Event trait from aletheon-abi and adds comm-specific
//! event utilities.

pub use aletheon_abi::{Event, EventType, Priority};

use std::any:: Any;

/// A concrete event implementation for direct use.
pub struct ConcreteEvent {
    event_type: EventType,
    priority: Priority,
    source: String,
    payload: Box<dyn Any + Send + Sync>,
}

impl ConcreteEvent {
    pub fn new(
        event_type: EventType,
        priority: Priority,
        source: String,
        payload: Box<dyn Any + Send + Sync>,
    ) -> Self {
        Self { event_type, priority, source, payload }
    }
}

impl Event for ConcreteEvent {
    fn event_type(&self) -> EventType { self.event_type.clone() }
    fn priority(&self) -> Priority { self.priority }
    fn source(&self) -> &str { &self.source }
    fn payload(&self) -> &dyn Any { &*self.payload }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concrete_event() {
        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::High,
            "test".to_string(),
            Box::new("payload"),
        );
        assert_eq!(event.event_type(), EventType::UserIntent);
        assert_eq!(event.priority(), Priority::High);
        assert_eq!(event.source(), "test");
        assert!(event.payload().downcast_ref::<&str>().is_some());
    }

    #[test]
    fn test_concrete_event_summary() {
        let event = ConcreteEvent::new(
            EventType::ToolError,
            Priority::Critical,
            "body".to_string(),
            Box::new(42u32),
        );
        let summary = event.summary();
        assert!(summary.contains("ToolError"));
        assert!(summary.contains("body"));
    }
}
