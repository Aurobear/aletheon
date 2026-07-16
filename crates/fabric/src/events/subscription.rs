use std::{collections::HashMap, future::Future, pin::Pin};

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{EnvelopeV2, SchemaId, SubscriptionId};

pub type EnvelopeHandler = Box<dyn Fn(&EnvelopeV2) -> bool + Send + Sync>;
pub type AsyncEnvelopeHandler =
    Box<dyn Fn(EnvelopeV2) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

pub struct Subscription {
    pub id: SubscriptionId,
    pub schema: SchemaId,
    pub handler: EnvelopeHandler,
}

pub struct SubscriptionRegistry {
    subscriptions: RwLock<HashMap<SchemaId, Vec<Subscription>>>,
    next_id: AtomicU64,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self {
            subscriptions: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn subscribe(&self, schema: SchemaId, handler: EnvelopeHandler) -> SubscriptionId {
        let id = SubscriptionId(self.next_id.fetch_add(1, Ordering::SeqCst));
        self.subscriptions
            .write()
            .entry(schema.clone())
            .or_default()
            .push(Subscription {
                id,
                schema,
                handler,
            });
        id
    }

    pub fn unsubscribe(&self, id: SubscriptionId) -> bool {
        for handlers in self.subscriptions.write().values_mut() {
            if let Some(position) = handlers
                .iter()
                .position(|subscription| subscription.id == id)
            {
                handlers.remove(position);
                return true;
            }
        }
        false
    }

    pub fn dispatch(&self, envelope: &EnvelopeV2) -> bool {
        if let Some(handlers) = self.subscriptions.read().get(&envelope.schema) {
            for subscription in handlers {
                if !(subscription.handler)(envelope) {
                    return false;
                }
            }
        }
        true
    }

    pub fn has_subscribers(&self, schema: &SchemaId) -> bool {
        self.subscriptions
            .read()
            .get(schema)
            .is_some_and(|subscriptions| !subscriptions.is_empty())
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
