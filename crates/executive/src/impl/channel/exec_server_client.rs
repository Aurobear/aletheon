//! ExecServerClient — daemon-side adapter for exec-server JSON-RPC transport.
//!
//! Spawns the exec-server binary as a child process and communicates
//! via JSON-RPC over stdin/stdout. Gate: grok_hardening.streaming_tools
//! (future wiring).

use std::time::Duration;
use anyhow::Result;
use tokio::process::Child;

/// Configuration for spawning and communicating with the exec-server process.
#[allow(dead_code)]
pub(crate) struct ExecServerConfig {
    pub binary_path: String,
    pub shared_secret: String,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

/// Wraps a spawned exec-server child process and provides JSON-RPC
/// request/response communication over stdin/stdout.
#[allow(dead_code)]
pub(crate) struct ExecServerClient {
    child: Child,
    // stdin writer, stdout reader for JSON-RPC
}

#[allow(dead_code)]
impl ExecServerClient {
    /// Spawn the exec-server and perform the handshake.
    pub async fn spawn(config: ExecServerConfig) -> Result<Self> {
        let _ = config;
        // 1. Spawn the binary
        // 2. Send handshake with shared_secret
        // 3. Read handshake response (protocol_version, server_pid)
        // 4. Return client
        todo!("D1-T9: ExecServerClient spawn")
    }

    /// Send a JSON-RPC request and await the response.
    pub async fn request(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let _ = (method, params);
        todo!("D1-T9: ExecServerClient request")
    }

    /// Graceful shutdown: send shutdown, then wait with timeout.
    pub async fn shutdown(&mut self) -> Result<()> {
        todo!("D1-T9: ExecServerClient shutdown")
    }
}
