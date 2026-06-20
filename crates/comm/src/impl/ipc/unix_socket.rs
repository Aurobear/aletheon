//! Unix socket IPC backend (Tier 1 — always available).
//!
//! NOTE: This implements the low-level `IpcBackend` trait with `AgentMessage` + bincode.
//! For the high-level `Transport` trait with `Envelope` + serde_json, see
//! `unix_socket_transport::UnixSocketTransport`. The two intentionally serve
//! different trait hierarchies and are not duplicates.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use base::ipc_types::{AgentId, AgentMessage, IpcBackend, IpcProbeError};

/// Default socket directory.
const DEFAULT_SOCKET_DIR: &str = "/tmp/agent-ipc";

/// Default channel capacity per agent.
const DEFAULT_CHANNEL_CAP: usize = 256;

/// Maximum message size (1 MiB).
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// Unix socket IPC backend (Tier 1 — always available).
pub struct UnixSocketBackend {
    socket_path: PathBuf,
    /// Per-agent inbound message channels (writer side for the listener task).
    senders: Arc<RwLock<HashMap<AgentId, mpsc::Sender<AgentMessage>>>>,
    /// Set of registered agent IDs (reliable even after receiver is handed out).
    registered: Arc<RwLock<HashSet<AgentId>>>,
    /// Handle for the listener task so we can shut it down.
    listener_handle: Option<tokio::task::JoinHandle<()>>,
    /// Whether `init` has been called.
    initialized: bool,
}

impl UnixSocketBackend {
    /// Create a new backend with a custom socket directory.
    pub fn new() -> Self {
        Self::with_socket_dir(PathBuf::from(DEFAULT_SOCKET_DIR))
    }

    /// Create a new backend with a specific socket directory.
    pub fn with_socket_dir(socket_dir: PathBuf) -> Self {
        let socket_path = socket_dir.join("agent_ipc.sock");
        Self {
            socket_path,
            senders: Arc::new(RwLock::new(HashMap::new())),
            registered: Arc::new(RwLock::new(HashSet::new())),
            listener_handle: None,
            initialized: false,
        }
    }

    /// Register an agent and obtain its message receiver.
    ///
    /// Must be called **after** `init`.  Returns `None` if the agent is
    /// already registered (callers should drain the existing receiver
    /// instead).
    pub async fn register_agent(&self, agent_id: AgentId) -> Option<mpsc::Receiver<AgentMessage>> {
        {
            let reg = self.registered.read().await;
            if reg.contains(&agent_id) {
                return None;
            }
        }
        let (tx, rx) = mpsc::channel(DEFAULT_CHANNEL_CAP);
        self.senders.write().await.insert(agent_id, tx);
        self.registered.write().await.insert(agent_id);
        Some(rx)
    }

    /// Remove an agent's registration, closing its channel.
    pub async fn unregister_agent(&self, agent_id: AgentId) {
        self.senders.write().await.remove(&agent_id);
        self.registered.write().await.remove(&agent_id);
    }

    /// Return the socket path.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Spawn the listener task that accepts connections and routes messages.
    async fn spawn_listener(&mut self) -> Result<(), IpcProbeError> {
        // Ensure parent directory exists.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                IpcProbeError::Other(format!("Failed to create socket directory: {}", e))
            })?;
        }

        // Remove stale socket file if present.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).map_err(|e| {
                IpcProbeError::Other(format!("Failed to remove stale socket: {}", e))
            })?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| IpcProbeError::Other(format!("Failed to bind unix socket: {}", e)))?;

        info!(path = %self.socket_path.display(), "Unix socket IPC listening");

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

    /// Handle a single client connection: read length-prefixed messages and
    /// route them to the appropriate agent channel.
    async fn handle_connection(
        mut stream: UnixStream,
        senders: Arc<RwLock<HashMap<AgentId, mpsc::Sender<AgentMessage>>>>,
    ) -> Result<(), anyhow::Error> {
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
                    "Message too large: {} bytes (max {})",
                    len,
                    MAX_MESSAGE_SIZE
                ));
            }

            // Read payload.
            let mut payload = vec![0u8; len];
            stream.read_exact(&mut payload).await?;

            // Deserialize.
            let msg: AgentMessage = bincode::deserialize(&payload)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize message: {}", e))?;

            // Route to target agent(s).
            let senders = senders.read().await;
            if msg.target_id == 0 {
                // Broadcast to all agents except sender.
                for (id, sender) in senders.iter() {
                    if *id != msg.sender_id {
                        let _ = sender.send(msg.clone()).await;
                    }
                }
            } else if let Some(sender) = senders.get(&msg.target_id) {
                let _ = sender.send(msg).await;
            } else {
                debug!(
                    target = msg.target_id,
                    "Target agent not found, dropping message"
                );
            }
        }

        Ok(())
    }

    /// Serialize and send a message to a connected stream with length-prefix framing.
    async fn write_message(
        stream: &mut UnixStream,
        msg: &AgentMessage,
    ) -> Result<(), anyhow::Error> {
        use tokio::io::AsyncWriteExt;

        let bytes = bincode::serialize(msg)
            .map_err(|e| anyhow::anyhow!("Failed to serialize message: {}", e))?;
        let len = (bytes.len() as u32).to_be_bytes();

        stream.write_all(&len).await?;
        stream.write_all(&bytes).await?;
        stream.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl IpcBackend for UnixSocketBackend {
    async fn init(&mut self) -> Result<(), IpcProbeError> {
        if self.initialized {
            return Ok(());
        }
        self.spawn_listener().await?;
        self.initialized = true;
        Ok(())
    }

    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| IpcProbeError::Other(format!("Connect failed: {}", e)))?;

        Self::write_message(&mut stream, message)
            .await
            .map_err(|e| IpcProbeError::Other(format!("Send failed: {}", e)))
    }

    async fn recv(&self) -> Result<AgentMessage, IpcProbeError> {
        // The trait-level `recv` has no agent context, so we cannot pick a
        // per-agent receiver here.  Callers that need per-agent recv should
        // use `register_agent` directly.
        Err(IpcProbeError::Other(
            "Use register_agent() for per-agent recv; trait recv() is not supported by UnixSocketBackend".to_string(),
        ))
    }

    async fn try_recv(&self) -> Option<AgentMessage> {
        None
    }

    fn is_available(&self) -> bool {
        true // Always available — Tier 1
    }

    fn name(&self) -> &str {
        "unix_socket"
    }
}

impl Drop for UnixSocketBackend {
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
    use base::ipc_types::{IpcPriority as Priority, MessageType};

    #[tokio::test]
    async fn test_register_and_send() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = UnixSocketBackend::with_socket_dir(dir.path().to_path_buf());
        backend.init().await.unwrap();

        // Register agent 1.
        let mut rx = backend
            .register_agent(1)
            .await
            .expect("agent already registered");

        // Duplicate registration returns None.
        assert!(backend.register_agent(1).await.is_none());

        // Build a message targeting agent 1.
        let msg = AgentMessage::new(
            2,
            1,
            MessageType::Direct,
            Priority::Normal,
            b"hello".to_vec(),
        );

        // Send via the socket.
        backend.send(&msg).await.expect("send failed");

        // Receive on agent 1's channel.
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed");

        assert_eq!(received.sender_id, 2);
        assert_eq!(received.target_id, 1);
        assert_eq!(received.payload, b"hello");

        backend.unregister_agent(1).await;
    }

    #[tokio::test]
    async fn test_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = UnixSocketBackend::with_socket_dir(dir.path().to_path_buf());
        backend.init().await.unwrap();

        let mut rx1 = backend.register_agent(1).await.unwrap();
        let mut rx2 = backend.register_agent(2).await.unwrap();

        // Broadcast (target_id == 0) from agent 3.
        let msg = AgentMessage::new(
            3,
            0,
            MessageType::Event,
            Priority::Normal,
            b"broadcast".to_vec(),
        );
        backend.send(&msg).await.unwrap();

        let r1 = tokio::time::timeout(std::time::Duration::from_secs(2), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r1.sender_id, 3);

        let r2 = tokio::time::timeout(std::time::Duration::from_secs(2), rx2.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r2.sender_id, 3);
    }
}
