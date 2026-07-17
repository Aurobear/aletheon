//! Schema-filtered, bounded in-process delivery for canonical envelopes.

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::events::subscription::{AsyncEnvelopeHandler, EnvelopeHandler, SubscriptionRegistry};
use crate::{EnvelopeV2, SchemaId, SubscriptionId};

pub struct KernelEventBus {
    subscriptions: SubscriptionRegistry,
    channels: RwLock<HashMap<SchemaId, broadcast::Sender<EnvelopeV2>>>,
    channel_capacity: usize,
}

impl KernelEventBus {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            subscriptions: SubscriptionRegistry::new(),
            channels: RwLock::new(HashMap::new()),
            channel_capacity: channel_capacity.max(1),
        }
    }

    pub async fn publish(&self, envelope: EnvelopeV2) -> Result<()> {
        envelope.validate_known_schema()?;
        self.subscriptions.dispatch(&envelope);
        if let Some(channel) = self.channels.read().get(&envelope.schema) {
            // A lagging receiver observes `RecvError::Lagged`; overload is not
            // silently converted into an unbounded allocation.
            let _ = channel.send(envelope);
        }
        Ok(())
    }

    pub fn subscribe_channel(&self, schema: SchemaId) -> broadcast::Receiver<EnvelopeV2> {
        self.channels
            .write()
            .entry(schema)
            .or_insert_with(|| broadcast::channel(self.channel_capacity).0)
            .subscribe()
    }

    pub async fn subscribe(
        &self,
        schema: SchemaId,
        handler: EnvelopeHandler,
    ) -> Result<SubscriptionId> {
        schema.validate_known()?;
        let id = self.subscriptions.subscribe(schema, handler);
        debug!(subscription_id = id.0, "new envelope subscription");
        Ok(id)
    }

    pub async fn subscribe_async(
        &self,
        schema: SchemaId,
        handler: AsyncEnvelopeHandler,
    ) -> Result<SubscriptionId> {
        let handler = Arc::new(handler);
        self.subscribe(
            schema,
            Box::new(move |envelope| {
                let envelope = envelope.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    handler(envelope).await;
                });
                true
            }),
        )
        .await
    }

    pub async fn unsubscribe(&self, id: SubscriptionId) -> Result<()> {
        if !self.subscriptions.unsubscribe(id) {
            warn!(subscription_id = id.0, "unsubscribe called for unknown ID");
        }
        Ok(())
    }

    pub async fn has_subscribers(&self, schema: &SchemaId) -> bool {
        self.subscriptions.has_subscribers(schema)
    }
}
