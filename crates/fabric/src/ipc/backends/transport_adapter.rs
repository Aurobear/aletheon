//! IpcBackendAdapter — wraps legacy IpcBackend as Transport.
//!
//! This adapter bridges the old `IpcBackend` trait (which uses `AgentMessage`)
//! with the new `Transport` trait (which uses `Envelope`).
//!
//! **Migration path:** Use `Transport` implementations directly for new code.

#![allow(deprecated)]

use anyhow::Result;
use async_trait::async_trait;

use crate::ipc::envelope::{Envelope, Target};
use crate::ipc::ipc_types::{AgentMessage, IpcBackend, IpcPriority, MessageType};
use crate::ipc::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

/// Adapter that wraps an `IpcBackend` as a `Transport`.
///
/// This allows legacy IPC backends (Unix socket, io_uring, shared memory)
/// to be used through the unified Transport interface.
pub struct IpcBackendAdapter {
    backend: Box<dyn IpcBackend>,
}

impl IpcBackendAdapter {
    /// Create a new adapter wrapping an IpcBackend.
    pub fn new(backend: Box<dyn IpcBackend>) -> Self {
        Self { backend }
    }

    /// Get a reference to the underlying backend.
    pub fn backend(&self) -> &dyn IpcBackend {
        &*self.backend
    }

    /// Convert an Envelope to an AgentMessage for IPC transmission.
    fn envelope_to_agent_message(envelope: &Envelope) -> AgentMessage {
        // Extract agent IDs from source and target
        let sender_id = match &envelope.source {
            crate::ipc::envelope::Endpoint::Agent(pid) => *pid,
            _ => 0, // System or Module sources use 0
        };

        let target_id = match &envelope.target {
            crate::ipc::envelope::Target::Agent(pid) => *pid,
            crate::ipc::envelope::Target::Broadcast => 0, // Broadcast
            _ => 0,
        };

        // Map envelope priority to IPC priority
        let priority = match envelope.priority {
            crate::events::types::Priority::Critical => IpcPriority::Urgent,
            crate::events::types::Priority::High => IpcPriority::ToolCall,
            crate::events::types::Priority::Normal => IpcPriority::Normal,
            crate::events::types::Priority::Low => IpcPriority::Background,
            crate::events::types::Priority::Background => IpcPriority::Batch,
        };

        // Serialize envelope as payload
        let payload = serde_json::to_vec(envelope).unwrap_or_default();

        AgentMessage::new(sender_id, target_id, MessageType::Event, priority, payload)
    }
}

#[async_trait]
impl Transport for IpcBackendAdapter {
    fn kind(&self) -> TransportKind {
        // Map IPC backend type to transport kind
        if self.backend.name().contains("unix") {
            TransportKind::UnixSocket
        } else if self.backend.name().contains("io_uring") {
            TransportKind::IoUring
        } else if self.backend.name().contains("shared") {
            TransportKind::SharedMemory
        } else {
            TransportKind::InProcess
        }
    }

    fn can_reach(&self, target: &Target) -> bool {
        // IPC backends can reach Agent targets
        matches!(target, Target::Agent(_) | Target::Broadcast)
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        let msg = Self::envelope_to_agent_message(&envelope);
        self.backend
            .send(&msg)
            .await
            .map_err(|e| anyhow::anyhow!("IPC send failed: {e:?}"))
    }

    fn health(&self) -> TransportHealth {
        TransportHealth {
            status: if self.backend.is_available() {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            latency_ms: 0,
            queue_depth: 0,
            error_rate: 0.0,
        }
    }
}
