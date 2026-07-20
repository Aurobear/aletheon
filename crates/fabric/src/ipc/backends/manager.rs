//! IPC manager with auto-detection, preference-based selection, and runtime fallback.
//!
//! **Migration path:** Use `CommunicationBus` with `Transport` implementations
//! for new code. The `IpcManager` is kept for backward compatibility.

#![allow(deprecated)]

use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::ipc::ipc_types::{AgentId, AgentMessage, IpcBackend, IpcPreference, IpcProbeError};
use crate::ipc::transport::Transport;

use super::io_uring::IoUringBackend;
use super::priority_queue::PriorityQueue;
use super::shared_mem::SharedMemBackend;
use super::transport_adapter::IpcBackendAdapter;
use super::unix_socket::UnixSocketBackend;

// ---------------------------------------------------------------------------
// Backend kind discriminator (runtime tag for the active selection).
// ---------------------------------------------------------------------------

/// Identifies which concrete backend is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcBackendKind {
    UnixSocket,
    IoUring,
    SharedMemory,
}

// ---------------------------------------------------------------------------
// Environment detection.
// ---------------------------------------------------------------------------

/// Detected host environment characteristics.
#[derive(Debug)]
pub struct Environment {
    pub is_container: bool,
    pub is_wsl: bool,
    pub kernel_version: String,
    pub has_io_uring: bool,
}

impl Environment {
    /// Probe the running host for container / WSL / kernel features.
    pub fn detect() -> Self {
        let is_container = std::path::Path::new("/.dockerenv").exists()
            || std::fs::read_to_string("/proc/1/cgroup")
                .map(|c| c.contains("docker") || c.contains("kubepods"))
                .unwrap_or(false);

        let is_wsl = std::fs::read_to_string("/proc/version")
            .map(|v| v.contains("WSL") || v.contains("microsoft"))
            .unwrap_or(false);

        let kernel_version =
            std::fs::read_to_string("/proc/version").unwrap_or_else(|_| "unknown".to_string());

        let version_str = kernel_version.split_whitespace().nth(2).unwrap_or("0.0.0");

        let parts: Vec<&str> = version_str.split('.').collect();
        let major = parts
            .first()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let minor = parts
            .get(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let has_io_uring = major > 5 || (major == 5 && minor >= 10);

        Self {
            is_container,
            is_wsl,
            kernel_version: version_str.to_string(),
            has_io_uring,
        }
    }
}

// ---------------------------------------------------------------------------
// IpcManager.
// ---------------------------------------------------------------------------

/// IPC manager -- selects a primary backend at construction time and keeps a
/// Unix-socket fallback for runtime error recovery.
pub struct IpcManager {
    /// The primary (preferred) backend.
    primary: Box<dyn IpcBackend>,
    /// Always-available fallback backend.
    fallback: Box<dyn IpcBackend>,
    /// Tag indicating which concrete type is behind `primary`.
    active_kind: IpcBackendKind,
    /// Priority queue for locally-enqueued messages.
    queue: PriorityQueue,
    /// Registered agent metadata.
    agents: HashMap<AgentId, String>,
    /// Environment snapshot taken at construction time.
    env: Environment,
    /// Socket directory for Unix socket IPC.
    socket_dir: PathBuf,
}

impl IpcManager {
    // ------------------------------------------------------------------
    // Constructors.
    // ------------------------------------------------------------------

    /// Create with auto-detection -- picks the best backend for the host.
    pub fn auto_detect(socket_dir: PathBuf) -> Self {
        let env = Environment::detect();
        info!(
            container = env.is_container,
            wsl = env.is_wsl,
            kernel = %env.kernel_version,
            io_uring = env.has_io_uring,
            "IPC environment detected"
        );

        let fallback = Self::make_unix_socket(PathBuf::from("/tmp"));

        if env.has_io_uring && !env.is_container && !env.is_wsl {
            let primary = Box::new(IoUringBackend::new());
            // Probe returns false if the kernel is actually too old or
            // io_uring setup fails; fall back immediately in that case.
            if primary.is_available() {
                info!("Selected io_uring IPC backend");
                return Self {
                    primary,
                    fallback,
                    active_kind: IpcBackendKind::IoUring,
                    queue: PriorityQueue::new(1024),
                    agents: HashMap::new(),
                    env,
                    socket_dir,
                };
            }
            warn!("io_uring probe failed despite kernel version, falling back to Unix socket");
        }

        info!("Selected Unix socket IPC backend");
        Self {
            primary: Self::make_unix_socket(socket_dir.clone()),
            fallback,
            active_kind: IpcBackendKind::UnixSocket,
            queue: PriorityQueue::new(1024),
            agents: HashMap::new(),
            env,
            socket_dir,
        }
    }

    /// Create with an explicit preference.
    ///
    /// Returns `Err` only when the requested backend is provably unavailable
    /// (e.g. `Require(IoUring)` on kernel < 5.10).
    pub fn with_preference(
        preference: IpcPreference,
        socket_dir: PathBuf,
    ) -> Result<Self, IpcProbeError> {
        match preference {
            IpcPreference::Auto => Ok(Self::auto_detect(socket_dir)),

            IpcPreference::IoUring => {
                let env = Environment::detect();
                if !env.has_io_uring {
                    return Err(IpcProbeError::Other(format!(
                        "io_uring requires kernel >= 5.10, found {}",
                        env.kernel_version
                    )));
                }
                let primary = Box::new(IoUringBackend::new());
                if !primary.is_available() {
                    return Err(IpcProbeError::NotSupported);
                }
                Ok(Self {
                    primary,
                    fallback: Self::make_unix_socket(PathBuf::from("/tmp")),
                    active_kind: IpcBackendKind::IoUring,
                    queue: PriorityQueue::new(1024),
                    agents: HashMap::new(),
                    env,
                    socket_dir,
                })
            }

            IpcPreference::UnixSocket => {
                let env = Environment::detect();
                Ok(Self {
                    primary: Self::make_unix_socket(socket_dir.clone()),
                    fallback: Self::make_unix_socket(PathBuf::from("/tmp")),
                    active_kind: IpcBackendKind::UnixSocket,
                    queue: PriorityQueue::new(1024),
                    agents: HashMap::new(),
                    env,
                    socket_dir,
                })
            }

            IpcPreference::SharedMemory => {
                let env = Environment::detect();
                let primary = Box::new(SharedMemBackend::new());
                if !primary.is_available() {
                    return Err(IpcProbeError::NotSupported);
                }
                Ok(Self {
                    primary,
                    fallback: Self::make_unix_socket(PathBuf::from("/tmp")),
                    active_kind: IpcBackendKind::SharedMemory,
                    queue: PriorityQueue::new(1024),
                    agents: HashMap::new(),
                    env,
                    socket_dir,
                })
            }
        }
    }

    // ------------------------------------------------------------------
    // Accessors.
    // ------------------------------------------------------------------

    /// The kind of backend currently selected as primary.
    pub fn active_backend(&self) -> IpcBackendKind {
        self.active_kind
    }

    /// The detected environment snapshot.
    pub fn environment(&self) -> &Environment {
        &self.env
    }

    /// Whether the primary backend reports itself as available.
    pub fn is_primary_available(&self) -> bool {
        self.primary.is_available()
    }

    // ------------------------------------------------------------------
    // Message operations with fallback.
    // ------------------------------------------------------------------

    /// Initialize both primary and fallback backends.
    ///
    /// Call this once before sending / receiving.
    pub async fn init(&mut self) -> Result<(), IpcProbeError> {
        // Primary may fail (e.g. io_uring setup error); log and continue
        // with fallback as the effective backend.
        match self.primary.init().await {
            Ok(()) => {
                debug!(backend = self.primary.name(), "Primary backend initialized");
            }
            Err(e) => {
                warn!(error = %e, backend = self.primary.name(), "Primary init failed, promoting fallback");
                self.primary = Self::make_unix_socket(PathBuf::from("/tmp"));
                self.active_kind = IpcBackendKind::UnixSocket;
            }
        }

        // Always initialize the fallback.
        self.fallback.init().await?;
        Ok(())
    }

    /// Send a message with automatic fallback on primary failure.
    pub async fn send_with_fallback(&self, msg: &AgentMessage) -> Result<(), IpcProbeError> {
        match self.primary.send(msg).await {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(
                    error = %e,
                    backend = self.primary.name(),
                    "Primary send failed, trying fallback"
                );
                self.fallback.send(msg).await
            }
        }
    }

    /// Receive a message with timeout.
    ///
    /// Uses the primary backend's `recv`. Because the trait `recv` has no
    /// timeout parameter, callers should wrap this with
    /// `tokio::time::timeout` at the call site when using the trait-level
    /// recv.
    pub async fn recv(&self) -> Result<AgentMessage, IpcProbeError> {
        self.primary.recv().await
    }

    /// Try to receive without blocking.
    pub async fn try_recv(&self) -> Option<AgentMessage> {
        self.primary.try_recv().await
    }

    // ------------------------------------------------------------------
    // Priority queue (local enqueue / dequeue).
    // ------------------------------------------------------------------

    /// Enqueue a message into the local priority queue.
    pub fn enqueue(&mut self, message: AgentMessage) {
        self.queue.push(message);
    }

    /// Dequeue the highest-priority message from the local queue.
    pub fn dequeue(&mut self) -> Option<AgentMessage> {
        self.queue.pop()
    }

    /// Current queue depth.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    // ------------------------------------------------------------------
    // Agent registry.
    // ------------------------------------------------------------------

    /// Register an agent name for routing / diagnostics.
    pub fn register_agent(&mut self, agent_id: AgentId, name: String) {
        self.agents.insert(agent_id, name);
    }

    // ------------------------------------------------------------------
    // Transport integration.
    // ------------------------------------------------------------------

    /// Get a Transport adapter for the primary backend.
    ///
    /// This allows the IPC manager to be used with the unified Transport
    /// interface for cross-process communication. Creates a fresh backend
    /// instance of the same kind as the active primary.
    pub fn as_transport(&self) -> Box<dyn Transport> {
        match self.active_kind {
            IpcBackendKind::UnixSocket => {
                // For Unix socket, use the dedicated UnixSocketTransport
                // which has proper Envelope-level routing
                let socket_dir = self.socket_dir();
                Box::new(crate::ipc::transport::unix_socket_transport::UnixSocketTransport::with_socket_dir(socket_dir))
            }
            IpcBackendKind::IoUring => {
                Box::new(IpcBackendAdapter::new(Box::new(IoUringBackend::new())))
            }
            IpcBackendKind::SharedMemory => {
                Box::new(IpcBackendAdapter::new(Box::new(SharedMemBackend::new())))
            }
        }
    }

    /// Get the socket directory used by this manager.
    fn socket_dir(&self) -> PathBuf {
        self.socket_dir.clone()
    }

    // ------------------------------------------------------------------
    // Internals.
    // ------------------------------------------------------------------

    fn make_unix_socket(socket_dir: PathBuf) -> Box<dyn IpcBackend> {
        Box::new(UnixSocketBackend::with_socket_dir(socket_dir))
    }
}
