//! BusHandle — minimal trait for the communication bus.
//!
//! Subsystems receive an `Arc<dyn BusHandle>` via `SubsystemContext` so they can
//! register agents, publish envelopes, and make request-response calls without
//! depending on the concrete `CommunicationBus` type (which lives in fabric's
//! `ipc/` layer and carries heavy dependencies).

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Minimal handle to the communication bus for use by subsystems.
///
/// This trait lives in `aletheon-abi` so the `Subsystem` trait can reference it
/// without depending on fabric's heavy `ipc/bus/` implementation.
///
/// The concrete `CommunicationBus` in fabric implements this trait.
#[async_trait]
pub trait BusHandle: Send + Sync {
    /// Register an agent endpoint on the bus.
    async fn register_agent(
        &self,
        agent_id: u64,
        endpoint: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<()>;

    /// Publish a message to subscribers.
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()>;

    /// Request-response: send and wait for reply.
    async fn request(&self, topic: &str, payload: &[u8]) -> Result<Vec<u8>>;
}
