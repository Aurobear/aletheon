use anyhow::Result;
use async_trait::async_trait;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use crate::r#impl::event_log::EventLog;
use crate::r#impl::routing_policy::{RouteAction, RoutingPolicy};
use crate::r#impl::subscription::SubscriptionRegistry;
use aletheon_abi::{AsyncEventHandler, Event, EventBus, EventHandler, EventType, SubscriptionId};

pub struct KernelEventBus {
    subscriptions: SubscriptionRegistry,
    event_log: Arc<RwLock<EventLog>>,
}

impl KernelEventBus {
    pub fn new(log_capacity: usize) -> Self {
        Self {
            subscriptions: SubscriptionRegistry::new(),
            event_log: Arc::new(RwLock::new(EventLog::new(log_capacity))),
        }
    }

    pub fn event_log(&self) -> Arc<RwLock<EventLog>> {
        self.event_log.clone()
    }
}

#[async_trait]
impl EventBus for KernelEventBus {
    async fn publish(&self, event: Box<dyn Event>) -> Result<()> {
        // 1. Record in event log
        self.event_log.write().record(&*event);

        // 2. Check routing policy
        let route = RoutingPolicy::evaluate(&event.event_type(), &event.priority());
        match route {
            RouteAction::RequireSelfFieldReview => {
                // Phase 1: log warning, still deliver (no actual SelfField gate yet)
                warn!(
                    event_type = ?event.event_type(),
                    source = event.source(),
                    "Event requires SelfField review (Phase 1: delivering anyway)"
                );
            }
            RouteAction::FastPath => {
                debug!(
                    event_type = ?event.event_type(),
                    source = event.source(),
                    "Event on fast path"
                );
            }
        }

        // 3. Dispatch to subscribers
        self.subscriptions.dispatch(&*event);

        Ok(())
    }

    async fn subscribe(
        &self,
        event_type: EventType,
        handler: EventHandler,
    ) -> Result<SubscriptionId> {
        let id = self.subscriptions.subscribe(event_type, handler);
        debug!(subscription_id = id.0, "New subscription");
        Ok(id)
    }

    async fn subscribe_async(
        &self,
        event_type: EventType,
        handler: AsyncEventHandler,
    ) -> Result<SubscriptionId> {
        let handler = Arc::new(handler);
        let sync_handler: EventHandler = Box::new(move |event: &dyn Event| {
            let json = event.to_json();
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(json).await;
            });
            true // non-blocking, continue propagation
        });
        let id = self.subscriptions.subscribe(event_type, sync_handler);
        debug!(subscription_id = id.0, "New async subscription");
        Ok(id)
    }

    async fn request(&self, event: Box<dyn Event>, timeout: Duration) -> Result<Box<dyn Event>> {
        // Phase 1: request-response not fully implemented.
        // For now, publish the event and return error after timeout.
        // Full implementation will use oneshot channels when response events are supported.
        warn!("request() not fully implemented in Phase 1; publishing event only");
        self.publish(event).await?;
        tokio::time::sleep(timeout).await;
        Err(anyhow::anyhow!(
            "Request timeout — no response received (Phase 1 limitation)"
        ))
    }

    async fn unsubscribe(&self, id: SubscriptionId) -> Result<()> {
        let found = self.subscriptions.unsubscribe(id);
        if found {
            debug!(subscription_id = id.0, "Unsubscribed");
        } else {
            warn!(subscription_id = id.0, "Unsubscribe called for unknown ID");
        }
        Ok(())
    }

    async fn has_subscribers(&self, event_type: &EventType) -> bool {
        self.subscriptions.has_subscribers(event_type)
    }
}
