//! EventBus trait — like Linux kernel's interrupt controller.
//!
//! The EventBus is the central message router. All cross-subsystem
//! communication flows through it. Three communication patterns:
//! - Publish-Subscribe (one-to-many broadcast)
//! - Request-Response (synchronous wait for reply)
//! - Fire-and-Forget (async, no wait)

use anyhow::Result;
use async_trait::async_trait;
use std::time::Duration;

use crate::event::{AsyncEventHandler, Event, EventHandler, EventType, SubscriptionId};

/// EventBus trait — the interrupt controller of Aletheon.
///
/// Subsystems subscribe to event types and receive callbacks when
/// events are published. The EventBus handles routing, priority
/// ordering, and delivery.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish an event. All matching subscribers are notified.
    ///
    /// Events are delivered in priority order (Critical first).
    /// Returns after all synchronous handlers have completed.
    async fn publish(&self, event: Box<dyn Event>) -> Result<()>;

    /// Subscribe to an event type. Returns a subscription ID for later removal.
    ///
    /// The handler is called whenever an event of the specified type is published.
    /// If the handler returns `false`, no further handlers for that event are called
    /// (early termination / "handled" semantics).
    async fn subscribe(
        &self,
        event_type: EventType,
        handler: EventHandler,
    ) -> Result<SubscriptionId>;

    /// Request-Response pattern. Publishes an event and waits for a reply.
    ///
    /// Like Linux ioctl — synchronous call that blocks until the handler
    /// produces a response or timeout is reached.
    async fn request(&self, event: Box<dyn Event>, timeout: Duration) -> Result<Box<dyn Event>>;

    /// Unsubscribe a previously registered handler.
    async fn unsubscribe(&self, id: SubscriptionId) -> Result<()>;

    /// Check if any subscribers exist for an event type.
    async fn has_subscribers(&self, event_type: &EventType) -> bool;

    /// Subscribe with an async handler.
    ///
    /// Default implementation wraps the async handler into a synchronous
    /// `EventHandler` by spawning the future on the Tokio runtime. The
    /// sync wrapper always returns `true` (continues propagation) because
    /// the async result is not awaited synchronously.
    ///
    /// Implementations that support native async dispatch should override
    /// this method.
    async fn subscribe_async(
        &self,
        event_type: EventType,
        handler: AsyncEventHandler,
    ) -> Result<SubscriptionId> {
        let sync_handler: EventHandler = Box::new(move |event: &dyn Event| {
            let json = event.to_json();
            let fut = handler(json);
            tokio::spawn(fut);
            true // cannot await here; propagate unconditionally
        });
        self.subscribe(event_type, sync_handler).await
    }
}
