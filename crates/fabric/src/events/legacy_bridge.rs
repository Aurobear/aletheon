//! LegacyEventBridge — adapts old Event/EventBus subscribers to EnvelopeV2.
//!
//! This is the migration bridge documented in `docs/arch/04_COMMUNICATION_FABRIC_V2.md`.
//! It wraps a `CommunicationBus` and translates old `EventBus::subscribe()` /
//! `EventBus::publish()` calls into `EnvelopeV2` topic-based publish/subscribe.
//!
//! # Usage
//!
//! ```ignore
//! let comm_bus = Arc::new(CommunicationBus::new());
//! let bridge = LegacyEventBridge::new(comm_bus);
//!
//! // Old-style subscription — now routed through EnvelopeV2 topics.
//! let sid = bridge.subscribe(EventType::ToolObservation, Box::new(|event| {
//!     println!("tool observation: {:?}", event.summary());
//!     true
//! })).await?;
//! ```
//!
//! # Migration path
//!
//! 1. Replace `Arc<dyn EventBus>` with `Arc<LegacyEventBridge>` in struct fields.
//! 2. Replace `event_bus.subscribe(...)` with `bridge.subscribe(...)`.
//! 3. Eventually replace `LegacyEventBridge::subscribe(EventType, callback)` with
//!    direct `CommunicationBus::subscribe_topic(schema_id, buffer)` calls.
//! 4. Delete `LegacyEventBridge` when all subscribers are migrated.

#![allow(deprecated)]

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::events::event::{Event, EventType, Priority};
use crate::include::event_bus::EventBus;
use crate::ipc::bus::communication_bus::CommunicationBus;
use crate::ipc::envelope::{Envelope, Payload, Pattern, Target};
use crate::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId};
use crate::{EventHandler, SubscriptionId};

// ---------------------------------------------------------------------------
// LegacyEventBridge
// ---------------------------------------------------------------------------

/// Adapter that bridges old `EventBus` subscribers to the new `EnvelopeV2`
/// topic-based communication fabric.
///
/// Each old `EventType` variant maps to a `SchemaId` topic. Subscriptions
/// register through the `CommunicationBus` topic system, and `publish()`
/// converts old `Event` objects to `Envelope` envelopes for routing.
pub struct LegacyEventBridge {
    comm: Arc<CommunicationBus>,
    /// Active subscription ids with their schema topics.
    topics: Mutex<Vec<(SubscriptionId, String)>>,
    next_sub_id: Mutex<u64>,
}

impl LegacyEventBridge {
    /// Create a new bridge wrapping the given `CommunicationBus`.
    pub fn new(comm: Arc<CommunicationBus>) -> Self {
        Self {
            comm,
            topics: Mutex::new(Vec::new()),
            next_sub_id: Mutex::new(1),
        }
    }

    /// Map an `EventType` to the `SchemaId` topic used for V2 routing.
    pub fn schema_for(event_type: &EventType) -> SchemaId {
        SchemaId::from(SchemaId::from_event_type(event_type))
    }

    /// Publish an old-style `Event` through the `CommunicationBus` using
    /// `EnvelopeV2` semantics.
    ///
    /// The event is converted to an `Envelope` with schema derived from
    /// `EventType` and published via `CommunicationBus::publish()`.
    pub async fn publish_v2(&self, event: Box<dyn Event>) -> Result<()> {
        let schema_str = Self::schema_for(&event.event_type()).to_string();
        let payload = event.to_json();
        let envelope = Envelope::new(
            crate::Endpoint::System,
            Target::Topic(schema_str),
            Pattern::Publish,
            Payload::Json(payload),
        )
        .with_priority(event.priority());

        self.comm.publish(envelope).await
    }

    /// Convert an `Event` into an `EnvelopeV2` for direct V2 routing.
    ///
    /// This is useful for code transitioning from Event-based publishing
    /// to EnvelopeV2-based publishing: it produces a fully-formed `EnvelopeV2`
    /// that can be sent through `MailboxService`.
    pub fn event_to_envelope_v2(
        &self,
        event: Box<dyn Event>,
        source: impl Into<String>,
    ) -> EnvelopeV2 {
        let schema = Self::schema_for(&event.event_type());
        EnvelopeV2::new(
            schema,
            crate::ipc::envelope_v2::Target(source.into()),
            crate::ipc::envelope_v2::Target("broadcast".into()),
            DeliveryPattern::FanOut,
            crate::NamespaceId("legacy".into()),
            event.to_json(),
        )
        .with_priority(event.priority().into_u8())
    }
}

// Priority → u8 conversion (reused from envelope_v2.rs)
trait PriorityU8 {
    fn into_u8(self) -> u8;
}

impl PriorityU8 for Priority {
    fn into_u8(self) -> u8 {
        match self {
            Priority::Critical => 255,
            Priority::High => 200,
            Priority::Normal => 128,
            Priority::Low => 50,
            Priority::Background => 10,
        }
    }
}

// ---------------------------------------------------------------------------
// EventBus impl — bridges old subscribe/publish to communication bus
// ---------------------------------------------------------------------------

#[async_trait]
impl EventBus for LegacyEventBridge {
    async fn publish(&self, event: Box<dyn Event>) -> Result<()> {
        self.publish_v2(event).await
    }

    async fn subscribe(
        &self,
        event_type: EventType,
        handler: EventHandler,
    ) -> Result<SubscriptionId> {
        let schema = Self::schema_for(&event_type);
        let topic = schema.0.clone();
        let mut rx = self.comm.subscribe_topic(&topic, Some(256));

        let id = {
            let mut n = self.next_sub_id.lock().await;
            let id = SubscriptionId(*n);
            *n += 1;
            id
        };

        // Track the subscription before spawning the background task.
        {
            let mut topics = self.topics.lock().await;
            topics.push((id, topic.clone()));
        }

        // Spawn a background task that reads topic messages and dispatches
        // to the old-style handler.
        let h = Arc::new(Mutex::new(handler));
        tokio::spawn(async move {
            while let Ok(envelope) = rx.recv().await {
                let json = match &envelope.payload {
                    Payload::Json(v) => v.clone(),
                    _ => continue,
                };
                let event = EventFromJson {
                    event_type: event_type.clone(),
                    priority: crate::events::event::Priority::Normal,
                    source: "legacy-bridge".to_string(),
                    json,
                };
                let guard = h.lock().await;
                if !guard(&event) {
                    break;
                }
            }
        });

        Ok(id)
    }

    async fn request(
        &self,
        _event: Box<dyn Event>,
        _timeout: std::time::Duration,
    ) -> Result<Box<dyn Event>> {
        Err(anyhow::anyhow!(
            "LegacyEventBridge::request is not supported — use CommunicationBus::request() with EnvelopeV2"
        ))
    }

    async fn unsubscribe(&self, id: SubscriptionId) -> Result<()> {
        let mut topics = self.topics.lock().await;
        topics.retain(|(sid, _)| *sid != id);
        Ok(())
    }

    async fn has_subscribers(&self, event_type: &EventType) -> bool {
        let schema = Self::schema_for(event_type);
        let topics = self.topics.lock().await;
        let schema_str = schema.0;
        topics.iter().any(|(_, topic)| topic == &schema_str)
    }
}

// ---------------------------------------------------------------------------
// EventFromJson — a lightweight Event impl for deserialized payloads
// ---------------------------------------------------------------------------

/// Lightweight `Event` implementation for payloads received via EnvelopeV2
/// topics. Satisfies the old `Event` trait so existing handlers work.
struct EventFromJson {
    event_type: EventType,
    priority: Priority,
    source: String,
    json: serde_json::Value,
}

impl Event for EventFromJson {
    fn event_type(&self) -> EventType {
        self.event_type.clone()
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn payload(&self) -> &dyn std::any::Any {
        &self.json
    }

    fn to_json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::event::EventType;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn schema_for_all_event_types_is_unique() {
        use std::collections::HashSet;
        let variants = [
            EventType::UserIntent,
            EventType::ToolObservation,
            EventType::AgentStarted,
            EventType::MemoryStored,
            EventType::PlanGenerated,
            EventType::SubsystemFailed,
            EventType::CognitivePulse,
        ];
        let mut seen = HashSet::new();
        for v in &variants {
            let schema = LegacyEventBridge::schema_for(v);
            assert!(seen.insert(schema.0.clone()), "duplicate schema for {v:?}");
        }
    }

    #[tokio::test]
    async fn publish_v2_sends_through_communication_bus() {
        let comm = Arc::new(CommunicationBus::new());
        let bridge = LegacyEventBridge::new(comm.clone());

        // Subscribe to the topic for ToolObservation events.
        let schema = LegacyEventBridge::schema_for(&EventType::ToolObservation);
        let mut rx = comm.subscribe_topic(&schema.0, Some(64));

        // Publish an event.
        let event = Box::new(crate::ConcreteEvent::new(
            EventType::ToolObservation,
            Priority::High,
            "test".to_string(),
            Box::new(serde_json::json!({"ok": true})),
        ));
        bridge.publish_v2(event).await.unwrap();

        // Should arrive on the topic.
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout")
            .expect("recv failed");
        assert!(matches!(received.pattern, Pattern::Publish));
    }

    #[tokio::test]
    async fn eventbus_subscribe_then_publish_delivers_to_handler() {
        let comm = Arc::new(CommunicationBus::new());
        let bridge = Arc::new(LegacyEventBridge::new(comm.clone()));

        let received = Arc::new(AtomicUsize::new(0));
        let r = received.clone();

        // Subscribe through the EventBus trait.
        let sid = bridge
            .subscribe(EventType::UserIntent, Box::new(move |_event| {
                r.fetch_add(1, Ordering::SeqCst);
                true
            }))
            .await
            .unwrap();

        // Publish through the EventBus trait.
        let event: Box<dyn Event> = Box::new(crate::ConcreteEvent::new(
            EventType::UserIntent,
            Priority::Normal,
            "test".to_string(),
            Box::new(serde_json::json!({"msg": "hello"})),
        ));
        bridge.publish(event).await.unwrap();

        // Give the background dispatch task time to process.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(received.load(Ordering::SeqCst), 1, "handler was not called");

        // Unsubscribe cleanly.
        bridge.unsubscribe(sid).await.unwrap();
    }

    #[tokio::test]
    async fn event_to_envelope_v2_produces_valid_envelope() {
        let comm = Arc::new(CommunicationBus::new());
        let bridge = LegacyEventBridge::new(comm);

        let event: Box<dyn Event> = Box::new(crate::ConcreteEvent::new(
            EventType::AgentStarted,
            Priority::High,
            "kernel".to_string(),
            Box::new(serde_json::json!({"pid": 42})),
        ));

        let env = bridge.event_to_envelope_v2(event, "kernel");
        assert_eq!(env.schema.0, "aletheon.event.agent_started/v1");
        assert_eq!(env.priority, 200); // High
        assert_eq!(env.source.0, "kernel");
        assert!(matches!(env.pattern, DeliveryPattern::FanOut));
    }
}
