//! Transport trait and implementations.

pub mod unix_socket_transport;

use crate::ipc::envelope::{Envelope, Target};
use async_trait::async_trait;

/// Transport backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Intra-process channels (loopback).
    InProcess,
    /// Unix domain socket.
    UnixSocket,
    /// io_uring (future).
    IoUring,
    /// Shared memory (future).
    SharedMemory,
}

/// Transport health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Transport health report.
#[derive(Debug, Clone)]
pub struct TransportHealth {
    pub status: HealthStatus,
    pub latency_ms: u64,
    pub queue_depth: u32,
    pub error_rate: f64,
}

/// Transport trait — unified interface for all transport backends.
/// Analogous to Linux net_device.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Transport type identifier.
    fn kind(&self) -> TransportKind;

    /// Whether this transport can reach the target.
    fn can_reach(&self, target: &Target) -> bool;

    /// Send message (one-way, no response expected).
    async fn send(&self, envelope: Envelope) -> anyhow::Result<()>;

    /// Health status.
    fn health(&self) -> TransportHealth;
}
