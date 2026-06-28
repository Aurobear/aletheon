// crates/aletheon-abi/src/protocol.rs

use crate::ipc::envelope::Envelope;
use async_trait::async_trait;

/// Protocol trait — communication pattern abstraction.
/// Different patterns (request-response, pub-sub, stream) implement this.
#[async_trait]
pub trait Protocol: Send + Sync {
    /// Send a request and wait for a correlated response.
    async fn request(&self, envelope: Envelope) -> anyhow::Result<Envelope>;

    /// Publish an envelope (fire-and-forget or broadcast).
    async fn publish(&self, envelope: Envelope) -> anyhow::Result<()>;
}
