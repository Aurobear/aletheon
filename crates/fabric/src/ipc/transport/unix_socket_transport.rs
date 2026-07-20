//! UnixSocketTransport — Envelope-based transport over Unix domain sockets.
//!
//! Implements the `Transport` trait using Unix domain sockets with
//! `Envelope`-based messaging. Uses serde_json for serialization and
//! 4-byte big-endian length-prefixed framing.
//!
//! NOTE: This is the high-level `Transport` abstraction (Envelope + Target routing).
//! For the lower-level `IpcBackend` implementation using `AgentMessage` + bincode,
//! see `ipc/unix_socket::UnixSocketBackend`. The two intentionally serve different
//! trait hierarchies and are not duplicates.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use crate::ipc::envelope::*;
use crate::ipc::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

/// Default socket directory.
const DEFAULT_SOCKET_DIR: &str = "/tmp/agent-ipc";

/// Default channel capacity per agent.
const DEFAULT_CHANNEL_CAP: usize = 256;

/// Maximum message size (1 MiB).
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// Unix socket transport for Envelope-based IPC.
///
/// Each agent registers with its `Pid` and obtains a receiving half of an
/// `mpsc` channel. Other agents send envelopes addressed to that `Pid`.
pub struct UnixSocketTransport {
    socket_path: PathBuf,
    /// Per-agent inbound message channels (writer side for the listener task).
    senders: Arc<RwLock<HashMap<u64, mpsc::Sender<Envelope>>>>,
    /// Set of registered agent IDs.
    registered: Arc<RwLock<std::collections::HashSet<u64>>>,
    /// Handle for the listener task so we can shut it down.
    listener_handle: Option<tokio::task::JoinHandle<()>>,
    /// Whether the transport has been initialized.
    initialized: bool,
    /// Connection pool: maps socket path to reusable stream.
    pool: Arc<tokio::sync::Mutex<HashMap<PathBuf, UnixStream>>>,
    /// Maximum connections in pool.
    max_pool_size: usize,
}

impl Default for UnixSocketTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UnixSocketTransport {
    /// Create a new transport with the default socket directory.
    pub fn new() -> Self {
        Self::with_socket_dir(PathBuf::from(DEFAULT_SOCKET_DIR))
    }

    /// Create a new transport with a specific socket directory.
    pub fn with_socket_dir(socket_dir: PathBuf) -> Self {
        let socket_path = socket_dir.join("envelope_ipc.sock");
        Self {
            socket_path,
            senders: Arc::new(RwLock::new(HashMap::new())),
            registered: Arc::new(RwLock::new(std::collections::HashSet::new())),
            listener_handle: None,
            initialized: false,
            pool: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            max_pool_size: 8,
        }
    }

    /// Initialize the transport: bind the socket and spawn the listener task.
    pub async fn init(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        self.spawn_listener().await?;
        self.initialized = true;
        Ok(())
    }

    /// Get a connection from the pool or create a new one.
    async fn get_pooled_connection(&self, path: &std::path::Path) -> Result<UnixStream> {
        let mut pool = self.pool.lock().await;
        if let Some(stream) = pool.remove(path) {
            // Verify connection is still alive
            if stream.peer_addr().is_ok() {
                return Ok(stream);
            }
        }
        // Create new connection
        UnixStream::connect(path)
            .await
            .map_err(|e| anyhow::anyhow!("Connect failed: {}", e))
    }

    /// Return a connection to the pool.
    async fn return_to_pool(&self, path: PathBuf, stream: UnixStream) {
        let mut pool = self.pool.lock().await;
        if pool.len() < self.max_pool_size {
            pool.insert(path, stream);
        }
        // If pool is full, stream is dropped (connection closed)
    }

    /// Register an agent and obtain its message receiver.
    ///
    /// Returns `None` if the agent is already registered.
    pub async fn register_agent(&self, pid: u64) -> Option<mpsc::Receiver<Envelope>> {
        {
            let reg = self.registered.read().await;
            if reg.contains(&pid) {
                return None;
            }
        }
        let (tx, rx) = mpsc::channel(DEFAULT_CHANNEL_CAP);
        self.senders.write().await.insert(pid, tx);
        self.registered.write().await.insert(pid);
        Some(rx)
    }

    /// Remove an agent's registration, closing its channel.
    pub async fn unregister_agent(&self, pid: u64) {
        self.senders.write().await.remove(&pid);
        self.registered.write().await.remove(&pid);
    }

    /// Return the socket path.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Spawn the listener task that accepts connections and routes envelopes.
    async fn spawn_listener(&mut self) -> Result<()> {
        // Ensure parent directory exists.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Failed to create socket directory: {}", e))?;
        }

        // Remove stale socket file if present.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)
                .map_err(|e| anyhow::anyhow!("Failed to remove stale socket: {}", e))?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| anyhow::anyhow!("Failed to bind unix socket: {}", e))?;

        info!(path = %self.socket_path.display(), "Unix socket envelope transport listening");

        let senders = self.senders.clone();

        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _addr)) => {
                        let senders = senders.clone();
                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_connection(stream, senders).await {
                                warn!(error = %e, "Connection handler error");
                            }
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "Accept error");
                    }
                }
            }
        });

        self.listener_handle = Some(handle);
        Ok(())
    }

    /// Handle a single client connection: read length-prefixed envelopes and
    /// route them to the appropriate agent channel.
    async fn handle_connection(
        mut stream: UnixStream,
        senders: Arc<RwLock<HashMap<u64, mpsc::Sender<Envelope>>>>,
    ) -> Result<()> {
        use tokio::io::AsyncReadExt;

        loop {
            // Read length prefix (4 bytes, big-endian).
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_be_bytes(len_buf) as usize;

            if len > MAX_MESSAGE_SIZE {
                return Err(anyhow::anyhow!(
                    "Envelope too large: {} bytes (max {})",
                    len,
                    MAX_MESSAGE_SIZE
                ));
            }

            // Read JSON payload.
            let mut payload = vec![0u8; len];
            stream.read_exact(&mut payload).await?;

            // Deserialize from JSON.
            let envelope: Envelope = serde_json::from_slice(&payload)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize envelope: {}", e))?;

            // Route to target agent.
            match &envelope.target {
                Target::Agent(pid) => {
                    let senders = senders.read().await;
                    if let Some(sender) = senders.get(pid) {
                        let _ = sender.send(envelope).await;
                    } else {
                        debug!(target = pid, "Target agent not found, dropping envelope");
                    }
                }
                Target::Broadcast => {
                    let senders = senders.read().await;
                    for sender in senders.values() {
                        let _ = sender.send(envelope.clone()).await;
                    }
                }
                _ => {
                    debug!(
                        "Unhandled target type in UnixSocketTransport: {:?}",
                        envelope.target
                    );
                }
            }
        }

        Ok(())
    }

    /// Serialize and send an envelope to a connected stream with length-prefix framing.
    async fn write_envelope(stream: &mut UnixStream, envelope: &Envelope) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let bytes = serde_json::to_vec(envelope)
            .map_err(|e| anyhow::anyhow!("Failed to serialize envelope: {}", e))?;
        let len = (bytes.len() as u32).to_be_bytes();

        stream.write_all(&len).await?;
        stream.write_all(&bytes).await?;
        stream.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl Transport for UnixSocketTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::UnixSocket
    }

    fn can_reach(&self, target: &Target) -> bool {
        match target {
            Target::Agent(_pid) => {
                // We can reach agents that are registered on this transport.
                // Use a blocking check since DashMap or a synchronous read
                // would be needed. For now, assume we can reach Agent targets
                // if initialized (the listener will handle routing).
                self.initialized
            }
            Target::Broadcast => self.initialized,
            _ => false,
        }
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        // Check TTL
        if envelope.is_expired() {
            anyhow::bail!("envelope expired");
        }

        // Get connection from pool (or create new one)
        let mut stream = self.get_pooled_connection(&self.socket_path).await?;

        // Send the envelope
        let result = Self::write_envelope(&mut stream, &envelope).await;

        // Return connection to pool on success
        if result.is_ok() {
            self.return_to_pool(self.socket_path.clone(), stream).await;
        }

        result
    }

    fn health(&self) -> TransportHealth {
        TransportHealth {
            status: if self.initialized {
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

impl Drop for UnixSocketTransport {
    fn drop(&mut self) {
        // Abort the listener task on drop.
        if let Some(handle) = self.listener_handle.take() {
            handle.abort();
        }
        // Best-effort cleanup of socket file.
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(feature = "network-tests"), ignore)]
    #[tokio::test]
    async fn test_register_and_send_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let mut transport = UnixSocketTransport::with_socket_dir(dir.path().to_path_buf());
        transport.init().await.unwrap();

        // Register agent 1.
        let mut rx = transport
            .register_agent(1)
            .await
            .expect("agent already registered");

        // Duplicate registration returns None.
        assert!(transport.register_agent(1).await.is_none());

        // Build an envelope targeting agent 1.
        let envelope = Envelope::fire_and_forget(
            Endpoint::Agent(2),
            Target::Agent(1),
            Payload::Json(serde_json::json!({"msg": "hello"})),
        );

        // Send via the socket.
        transport.send(envelope).await.expect("send failed");

        // Give the listener a moment to process.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Receive on agent 1's channel.
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed");

        assert_eq!(received.source, Endpoint::Agent(2));
        assert_eq!(received.target, Target::Agent(1));

        transport.unregister_agent(1).await;
    }

    #[cfg_attr(not(feature = "network-tests"), ignore)]
    #[tokio::test]
    async fn test_can_reach() {
        let dir = tempfile::tempdir().unwrap();
        let mut transport = UnixSocketTransport::with_socket_dir(dir.path().to_path_buf());

        // Before init, cannot reach anything.
        assert!(!transport.can_reach(&Target::Agent(1)));

        transport.init().await.unwrap();

        // After init, can reach Agent and Broadcast targets.
        assert!(transport.can_reach(&Target::Agent(1)));
        assert!(transport.can_reach(&Target::Broadcast));

        // Cannot reach Module or Topic targets.
        assert!(!transport.can_reach(&Target::Module(ModuleId::Cognit)));
        assert!(!transport.can_reach(&Target::Topic("test".to_string())));
    }

    #[tokio::test]
    async fn test_kind() {
        let dir = tempfile::tempdir().unwrap();
        let transport = UnixSocketTransport::with_socket_dir(dir.path().to_path_buf());
        assert_eq!(transport.kind(), TransportKind::UnixSocket);
    }
}
