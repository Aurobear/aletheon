//! # Aletheon Comm
//!
//! Communication layer for the Aletheon runtime. Provides the event bus,
//! event log, routing policy, and transport abstractions.
//!
//! ## Architecture
//!
//! - `core/` — Abstract traits and types (Event, EventBus)
//! - `bridge/` — Adapters connecting core to impl
//! - `impl/` — Concrete implementations (KernelEventBus, EventLog, IPC)

pub mod bridge;
pub mod core;
#[path = "impl/mod.rs"]
pub mod r#impl;

// Re-exports from core
pub use crate::core::bus::{EventBus, EventHandler, SubscriptionId};
pub use crate::core::event::{ConcreteEvent, Event, EventType, Priority};

// Re-exports from impl
pub use crate::r#impl::communication_bus::{BusConfig, CommunicationBus};
pub use crate::r#impl::debug_bus::{
    DebugBusHook, EventFilter, EventRecorder, PerfCounter, PerfSnapshot, RecordingMeta,
};
pub use crate::r#impl::event_log::{EventLog, LogEntry};
pub use crate::r#impl::in_process::InProcessTransport;
pub use crate::r#impl::ipc::{
    Environment, IpcBackendKind, IpcManager, JsonRpcAdapter, PriorityQueue,
};
pub use crate::r#impl::kernel_bus::KernelEventBus;
pub use crate::r#impl::pubsub::PubSubProtocol;
pub use crate::r#impl::request_response::RequestResponseProtocol;
pub use crate::r#impl::routing_policy::{RouteAction, RoutingPolicy};
pub use crate::r#impl::subscription::SubscriptionRegistry;

// Re-export protocol types from aletheon-abi
pub use aletheon_abi::envelope;
pub use aletheon_abi::protocol;
pub use aletheon_abi::transport;

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_publish_triggers_handler() {
        let bus = KernelEventBus::new(1000);
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = count.clone();

        bus.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::High,
            "test".to_string(),
            Box::new("payload"),
        );
        bus.publish(Box::new(event)).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_publish_wrong_type_no_trigger() {
        let bus = KernelEventBus::new(1000);
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = count.clone();

        bus.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

        let event = ConcreteEvent::new(
            EventType::ToolError,
            Priority::Normal,
            "test".to_string(),
            Box::new("payload"),
        );
        bus.publish(Box::new(event)).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_multiple_handlers_all_called() {
        let bus = KernelEventBus::new(1000);
        let count = Arc::new(AtomicU64::new(0));

        for _ in 0..3 {
            let count_clone = count.clone();
            bus.subscribe(
                EventType::UserIntent,
                Box::new(move |_| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                    true
                }),
            )
            .await
            .unwrap();
        }

        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::Normal,
            "test".to_string(),
            Box::new(()),
        );
        bus.publish(Box::new(event)).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_unsubscribe_removes_handler() {
        let bus = KernelEventBus::new(1000);
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = count.clone();

        let id = bus
            .subscribe(
                EventType::UserIntent,
                Box::new(move |_| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                    true
                }),
            )
            .await
            .unwrap();

        bus.unsubscribe(id).await.unwrap();

        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::Normal,
            "test".to_string(),
            Box::new(()),
        );
        bus.publish(Box::new(event)).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_has_subscribers() {
        let bus = KernelEventBus::new(1000);
        assert!(!bus.has_subscribers(&EventType::UserIntent).await);

        bus.subscribe(EventType::UserIntent, Box::new(|_| true))
            .await
            .unwrap();
        assert!(bus.has_subscribers(&EventType::UserIntent).await);
        assert!(!bus.has_subscribers(&EventType::ToolError).await);
    }

    #[tokio::test]
    async fn test_event_log_records() {
        let bus = KernelEventBus::new(1000);
        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::High,
            "test".to_string(),
            Box::new(()),
        );
        bus.publish(Box::new(event)).await.unwrap();

        let log = bus.event_log();
        let guard = log.read();
        let entries = guard.recent(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, EventType::UserIntent);
        assert_eq!(entries[0].source, "test");
    }

    #[tokio::test]
    async fn test_early_termination() {
        let bus = KernelEventBus::new(1000);
        let first = Arc::new(AtomicBool::new(false));
        let second = Arc::new(AtomicBool::new(false));

        let first_clone = first.clone();
        bus.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                first_clone.store(true, Ordering::SeqCst);
                false
            }),
        )
        .await
        .unwrap();

        let second_clone = second.clone();
        bus.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                second_clone.store(true, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

        let event = ConcreteEvent::new(
            EventType::UserIntent,
            Priority::Normal,
            "test".to_string(),
            Box::new(()),
        );
        bus.publish(Box::new(event)).await.unwrap();

        assert!(first.load(Ordering::SeqCst));
        assert!(!second.load(Ordering::SeqCst));
    }
}
