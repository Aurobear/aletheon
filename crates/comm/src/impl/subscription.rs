use base::{Event, EventHandler, EventType, SubscriptionId};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Subscription {
    pub id: SubscriptionId,
    pub event_type: EventType,
    pub handler: EventHandler,
}

pub struct SubscriptionRegistry {
    subscriptions: RwLock<HashMap<EventType, Vec<Subscription>>>,
    next_id: AtomicU64,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self {
            subscriptions: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn subscribe(&self, event_type: EventType, handler: EventHandler) -> SubscriptionId {
        let id = SubscriptionId(self.next_id.fetch_add(1, Ordering::SeqCst));
        let sub = Subscription {
            id,
            event_type: event_type.clone(),
            handler,
        };
        self.subscriptions
            .write()
            .entry(event_type)
            .or_default()
            .push(sub);
        id
    }

    pub fn unsubscribe(&self, id: SubscriptionId) -> bool {
        let mut subs = self.subscriptions.write();
        for handlers in subs.values_mut() {
            if let Some(pos) = handlers.iter().position(|s| s.id == id) {
                handlers.remove(pos);
                return true;
            }
        }
        false
    }

    /// Call all handlers for an event. Returns false if propagation was stopped.
    pub fn dispatch(&self, event: &dyn Event) -> bool {
        let subs = self.subscriptions.read();
        if let Some(handlers) = subs.get(&event.event_type()) {
            for sub in handlers {
                if !(sub.handler)(event) {
                    return false; // handler stopped propagation
                }
            }
        }
        true
    }

    pub fn has_subscribers(&self, event_type: &EventType) -> bool {
        self.subscriptions
            .read()
            .get(event_type)
            .is_some_and(|v| !v.is_empty())
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::Priority;
    use std::any::Any;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn test_subscribe_returns_incrementing_ids() {
        let registry = SubscriptionRegistry::new();
        let id1 = registry.subscribe(EventType::UserIntent, Box::new(|_| true));
        let id2 = registry.subscribe(EventType::UserIntent, Box::new(|_| true));
        assert!(id2.0 > id1.0);
    }

    #[test]
    fn test_has_subscribers() {
        let registry = SubscriptionRegistry::new();
        assert!(!registry.has_subscribers(&EventType::UserIntent));
        registry.subscribe(EventType::UserIntent, Box::new(|_| true));
        assert!(registry.has_subscribers(&EventType::UserIntent));
        assert!(!registry.has_subscribers(&EventType::ToolError));
    }

    #[test]
    fn test_unsubscribe() {
        let registry = SubscriptionRegistry::new();
        let id = registry.subscribe(EventType::UserIntent, Box::new(|_| true));
        assert!(registry.has_subscribers(&EventType::UserIntent));
        assert!(registry.unsubscribe(id));
        assert!(!registry.has_subscribers(&EventType::UserIntent));
    }

    #[test]
    fn test_unsubscribe_nonexistent() {
        let registry = SubscriptionRegistry::new();
        assert!(!registry.unsubscribe(SubscriptionId(999)));
    }

    struct TestEvent;

    impl Event for TestEvent {
        fn event_type(&self) -> EventType {
            EventType::UserIntent
        }
        fn priority(&self) -> Priority {
            Priority::Normal
        }
        fn source(&self) -> &str {
            "test"
        }
        fn payload(&self) -> &dyn Any {
            &()
        }
    }

    #[test]
    fn test_dispatch_calls_handlers() {
        let registry = SubscriptionRegistry::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        registry.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                called_clone.store(true, Ordering::SeqCst);
                true
            }),
        );

        registry.dispatch(&TestEvent);
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_dispatch_early_termination() {
        let registry = SubscriptionRegistry::new();
        let first_called = Arc::new(AtomicBool::new(false));
        let second_called = Arc::new(AtomicBool::new(false));

        let first_clone = first_called.clone();
        registry.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                first_clone.store(true, Ordering::SeqCst);
                false // stop propagation
            }),
        );

        let second_clone = second_called.clone();
        registry.subscribe(
            EventType::UserIntent,
            Box::new(move |_| {
                second_clone.store(true, Ordering::SeqCst);
                true
            }),
        );

        registry.dispatch(&TestEvent);
        assert!(first_called.load(Ordering::SeqCst));
        assert!(!second_called.load(Ordering::SeqCst));
    }
}
