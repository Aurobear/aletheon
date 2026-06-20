//! EventBridge — adapts core Event/EventBus traits to impl backends.
//!
//! Provides convenience functions for creating events and publishing them
//! through the KernelEventBus.

use aletheon_abi::{Event, EventBus, EventType, Priority};
use anyhow::Result;

use crate::core::event::ConcreteEvent;
use crate::r#impl::kernel_bus::KernelEventBus;

/// Bridge between core event types and the KernelEventBus implementation.
pub struct EventBridge;

impl EventBridge {
    /// Create a new ConcreteEvent with the given parameters.
    pub fn create_event(
        event_type: EventType,
        priority: Priority,
        source: impl Into<String>,
        payload: Box<dyn std::any::Any + Send + Sync>,
    ) -> Box<dyn Event> {
        Box::new(ConcreteEvent::new(event_type, priority, source.into(), payload))
    }

    /// Create a JSON-typed event (most common use case).
    pub fn create_json_event(
        event_type: EventType,
        priority: Priority,
        source: impl Into<String>,
        json: serde_json::Value,
    ) -> Box<dyn Event> {
        Self::create_event(event_type, priority, source, Box::new(json))
    }

    /// Publish an event to the KernelEventBus.
    pub async fn publish(bus: &KernelEventBus, event: Box<dyn Event>) -> Result<()> {
        bus.publish(event).await
    }

    /// Create and publish in one step.
    pub async fn emit(
        bus: &KernelEventBus,
        event_type: EventType,
        priority: Priority,
        source: impl Into<String>,
        json: serde_json::Value,
    ) -> Result<()> {
        let event = Self::create_json_event(event_type, priority, source, json);
        Self::publish(bus, event).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_json_event() {
        let event = EventBridge::create_json_event(
            EventType::UserIntent,
            Priority::High,
            "test",
            serde_json::json!({"key": "value"}),
        );
        assert_eq!(event.event_type(), EventType::UserIntent);
        assert_eq!(event.priority(), Priority::High);
        assert_eq!(event.source(), "test");
    }

    #[tokio::test]
    async fn test_publish_and_emit() {
        let bus = KernelEventBus::new(64);

        // Test publish
        let event = EventBridge::create_json_event(
            EventType::ToolError,
            Priority::Normal,
            "test",
            serde_json::json!({"error": "something"}),
        );
        assert!(EventBridge::publish(&bus, event).await.is_ok());

        // Test emit (create + publish)
        assert!(EventBridge::emit(
            &bus,
            EventType::UserIntent,
            Priority::Low,
            "emit-test",
            serde_json::json!({"input": "hello"}),
        )
        .await
        .is_ok());
    }

    #[test]
    fn test_create_event_with_string_payload() {
        let event = EventBridge::create_event(
            EventType::EnvironmentChange,
            Priority::Critical,
            "system",
            Box::new("critical alert".to_string()),
        );
        assert_eq!(event.event_type(), EventType::EnvironmentChange);
        assert!(event.payload().downcast_ref::<String>().is_some());
    }
}
