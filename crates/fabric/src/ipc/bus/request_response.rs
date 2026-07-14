// crates/aletheon-comm/src/impl/request_response.rs

//! Request-Response protocol with real correlation.
//! Replaces the stub EventBus::request() implementation.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::oneshot;

use crate::ipc::envelope::*;
use crate::ipc::protocol::Protocol;
use crate::ipc::transport::Transport;

/// Request-Response protocol.
/// Correlates requests and responses via envelope ID.
pub struct RequestResponseProtocol {
    transport: Arc<dyn Transport>,
    pending: DashMap<u64, oneshot::Sender<Envelope>>,
    /// Optional clock for deterministic timeout tracking in tests.
    /// When `None`, falls back to `tokio::time::timeout`.
    clock: Option<Arc<dyn crate::Clock>>,
}

impl RequestResponseProtocol {
    /// Create a new RequestResponseProtocol.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            pending: DashMap::new(),
            clock: None,
        }
    }

    /// Attach a clock for deterministic timeout tracking in tests.
    pub fn with_clock(mut self, clock: Arc<dyn crate::Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Register a response handler for a pending request.
    /// Called internally when a Response envelope arrives.
    pub fn handle_response(&self, response: &Envelope) -> bool {
        if let Some(correlation_id) = response.correlation_id {
            if let Some((_, tx)) = self.pending.remove(&correlation_id) {
                return tx.send(response.clone()).is_ok();
            }
        }
        false
    }

    /// Get the number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[async_trait]
impl Protocol for RequestResponseProtocol {
    async fn request(&self, mut envelope: Envelope) -> Result<Envelope> {
        // Ensure this is a Request pattern
        let timeout = match &envelope.pattern {
            Pattern::Request { timeout_ms } => Duration::from_millis(*timeout_ms),
            _ => {
                // Force into Request pattern with default timeout
                envelope.pattern = Pattern::Request { timeout_ms: 30_000 };
                Duration::from_secs(30)
            }
        };

        // Register pending correlation
        let (tx, rx) = oneshot::channel();
        self.pending.insert(envelope.id, tx);

        // Send the request
        self.transport
            .send(envelope.clone())
            .await
            .inspect_err(|_| {
                self.pending.remove(&envelope.id);
            })?;

        // Wait for response with timeout
        let result = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| anyhow::anyhow!("request timeout"));
        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender dropped — response channel closed
                self.pending.remove(&envelope.id);
                anyhow::bail!("response channel closed for request {}", envelope.id)
            }
            Err(_) => {
                // Timeout
                self.pending.remove(&envelope.id);
                anyhow::bail!(
                    "request {} timed out after {}ms",
                    envelope.id,
                    timeout.as_millis()
                )
            }
        }
    }

    async fn publish(&self, envelope: Envelope) -> Result<()> {
        self.transport.send(envelope).await
    }
}
