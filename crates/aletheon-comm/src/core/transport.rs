//! # Transport Abstraction
//!
//! Trait abstracting IPC transport mechanisms for inter-process communication.

use async_trait::async_trait;
use anyhow::Result;
use aletheon_abi::{AgentMessage, IpcProbeError};

/// Transport layer abstraction for inter-process communication.
///
/// Implementations provide different IPC backends (Unix sockets, shared memory,
/// io_uring, etc.) behind a unified async interface.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Initialize the transport. Must be called before send/recv.
    async fn init(&mut self) -> Result<(), IpcProbeError>;

    /// Send a message to a target agent.
    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError>;

    /// Receive the next message (blocks until available).
    async fn recv(&self) -> Result<AgentMessage, IpcProbeError>;

    /// Try to receive a message without blocking.
    async fn try_recv(&self) -> Option<AgentMessage>;

    /// Whether this transport backend is currently available.
    fn is_available(&self) -> bool;

    /// Human-readable name of this transport backend.
    fn name(&self) -> &str;
}
