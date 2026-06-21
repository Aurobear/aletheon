// crates/aletheon-comm/src/impl/pubsub.rs

//! Publish-Subscribe protocol.
//! Wraps existing EventBus for backward-compatible event broadcast.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::ipc::envelope::*;
use crate::ipc::protocol::Protocol;
use crate::ipc::transport::Transport;

/// Publish-Subscribe protocol.
/// Delegates to the underlying Transport for delivery.
pub struct PubSubProtocol {
    transport: Arc<dyn Transport>,
}

impl PubSubProtocol {
    /// Create a new PubSubProtocol.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl Protocol for PubSubProtocol {
    async fn request(&self, envelope: Envelope) -> Result<Envelope> {
        // PubSub doesn't support request-response; delegate to transport
        self.transport.send(envelope).await?;
        anyhow::bail!(
            "PubSubProtocol does not support request-response; use RequestResponseProtocol instead"
        )
    }

    async fn publish(&self, envelope: Envelope) -> Result<()> {
        self.transport.send(envelope).await
    }
}
