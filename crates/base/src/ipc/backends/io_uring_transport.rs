//! Transport-level wrapper for IoUringBackend.

use anyhow::Result;
use async_trait::async_trait;

use crate::ipc::envelope::{Envelope, Target};
use crate::ipc::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

use crate::ipc::ipc_types::IpcBackend;

use super::io_uring::IoUringBackend;
use super::transport_adapter::IpcBackendAdapter;

/// Transport implementation backed by io_uring.
///
/// Wraps `IoUringBackend` and adapts it to the `Transport` trait
/// using `IpcBackendAdapter` for Envelope-to-AgentMessage conversion.
pub struct IoUringTransport {
    adapter: IpcBackendAdapter,
    available: bool,
}

impl IoUringTransport {
    pub fn new(backend: Box<IoUringBackend>) -> Self {
        let available = (*backend).is_available();
        Self {
            adapter: IpcBackendAdapter::new(backend),
            available,
        }
    }
}

#[async_trait]
impl Transport for IoUringTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::IoUring
    }

    fn can_reach(&self, _target: &Target) -> bool {
        self.available
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        self.adapter.send(envelope).await
    }

    fn health(&self) -> TransportHealth {
        if self.available {
            TransportHealth {
                status: HealthStatus::Healthy,
                latency_ms: 0,
                queue_depth: 0,
                error_rate: 0.0,
            }
        } else {
            TransportHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: 0,
                queue_depth: 0,
                error_rate: 1.0,
            }
        }
    }
}
