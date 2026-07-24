//! Transport-level wrapper for JSON-RPC adapter.
//!
//! Implements the `Transport` trait using JSON-RPC line-delimited protocol
//! over Unix domain sockets, bridging the `JsonRpcAdapter` into the
//! unified Transport system.

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tracing::debug;

use crate::ipc::envelope::{Envelope, Target};
use crate::ipc::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

/// Transport implementation using JSON-RPC over Unix sockets.
///
/// This wraps the JSON-RPC line-delimited protocol into the `Transport` trait,
/// allowing it to be used alongside other transports in the IPC system.
pub struct JsonRpcTransport {
    socket_path: PathBuf,
    initialized: bool,
}

impl JsonRpcTransport {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            initialized: false,
        }
    }

    pub async fn init(&mut self) -> Result<()> {
        self.initialized = true;
        Ok(())
    }
}

#[async_trait]
impl Transport for JsonRpcTransport {
    fn kind(&self) -> TransportKind {
        // JSON-RPC uses Unix sockets as underlying transport
        TransportKind::UnixSocket
    }

    fn can_reach(&self, _target: &Target) -> bool {
        self.initialized
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        if !self.initialized {
            anyhow::bail!("JSON-RPC transport not initialized");
        }

        // Serialize envelope to JSON
        let json = serde_json::to_vec(&envelope)?;

        // Connect and send as line-delimited JSON
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| anyhow::anyhow!("JSON-RPC connect failed: {e}"))?;

        stream.write_all(&json).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        debug!(
            path = %self.socket_path.display(),
            bytes = json.len(),
            "JSON-RPC transport sent envelope"
        );

        Ok(())
    }

    fn health(&self) -> TransportHealth {
        if self.initialized {
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
