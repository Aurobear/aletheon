//! Daemon startup orchestration shared by all client adapters.
//!
//! The application service owns startup ordering and typed outcomes. Host
//! adapters own filesystem locks, service activation, process spawning, socket
//! probing, and diagnostic collection.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonInstallMode {
    SystemInstall,
    UserLocal,
    DevForeground,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstallModeFacts {
    pub system_install_available: bool,
    pub user_local_install_available: bool,
}

pub fn resolve_install_mode(facts: InstallModeFacts) -> DaemonInstallMode {
    if facts.system_install_available {
        DaemonInstallMode::SystemInstall
    } else if facts.user_local_install_available {
        DaemonInstallMode::UserLocal
    } else {
        DaemonInstallMode::DevForeground
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonReadiness {
    Absent,
    StaleSocket {
        detail: String,
    },
    Ready {
        protocol_version: u16,
        runtime_version: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonActivation {
    AlreadyRunning,
    ActivatedService,
    SpawnedForeground,
}

#[derive(Debug, Clone)]
pub struct EnsureDaemonRequest {
    pub socket: PathBuf,
    pub mode: DaemonInstallMode,
    pub startup_timeout: Duration,
    pub poll_interval: Duration,
    pub expected_protocol_version: u16,
    pub expected_runtime_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonReadyReceipt {
    pub mode: DaemonInstallMode,
    pub activation: DaemonActivation,
    pub protocol_version: u16,
    pub runtime_version: String,
    pub waited_ms: u64,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum DaemonLifecycleError {
    #[error("daemon startup lock timed out after {timeout_ms}ms")]
    LockTimeout { timeout_ms: u64 },
    #[error("daemon readiness probe failed: {0}")]
    Probe(String),
    #[error("stale daemon socket recovery failed: {0}")]
    StaleSocketRecovery(String),
    #[error("daemon activation failed: {0}")]
    Activation(String),
    #[error("daemon protocol mismatch: client={expected}, daemon={actual}")]
    ProtocolMismatch { expected: u16, actual: u16 },
    #[error("daemon runtime version mismatch: client={expected}, daemon={actual}")]
    RuntimeVersionMismatch { expected: String, actual: String },
    #[error("daemon was not ready after {timeout_ms}ms: {diagnostic}")]
    ReadinessTimeout {
        timeout_ms: u64,
        last_readiness: DaemonReadiness,
        diagnostic: String,
    },
}

/// Opaque startup lease. Dropping it releases the host-owned lock.
pub trait StartupLease: Send {}

#[async_trait::async_trait]
pub trait StartupLockPort: Send + Sync {
    async fn acquire(
        &self,
        timeout: Duration,
    ) -> Result<Box<dyn StartupLease>, DaemonLifecycleError>;
}

#[async_trait::async_trait]
pub trait DaemonLifecycleBackend: Send + Sync {
    async fn probe(
        &self,
        socket: &std::path::Path,
    ) -> Result<DaemonReadiness, DaemonLifecycleError>;

    async fn recover_stale_socket(
        &self,
        socket: &std::path::Path,
    ) -> Result<(), DaemonLifecycleError>;

    async fn activate(
        &self,
        mode: DaemonInstallMode,
        socket: &std::path::Path,
    ) -> Result<DaemonActivation, DaemonLifecycleError>;

    async fn diagnose(&self, socket: &std::path::Path) -> String;
}

pub struct DaemonLifecycleService {
    backend: Arc<dyn DaemonLifecycleBackend>,
    startup_lock: Arc<dyn StartupLockPort>,
}

impl DaemonLifecycleService {
    pub fn new(
        backend: Arc<dyn DaemonLifecycleBackend>,
        startup_lock: Arc<dyn StartupLockPort>,
    ) -> Self {
        Self {
            backend,
            startup_lock,
        }
    }

    pub async fn ensure_running(
        &self,
        request: EnsureDaemonRequest,
    ) -> Result<DaemonReadyReceipt, DaemonLifecycleError> {
        let started = Instant::now();
        if let Some(ready) = validate_ready(&request, self.backend.probe(&request.socket).await?)? {
            return Ok(receipt(
                &request,
                DaemonActivation::AlreadyRunning,
                ready,
                started,
            ));
        }

        let _lease = self.startup_lock.acquire(request.startup_timeout).await?;

        // A second client may have completed activation while this client was
        // waiting for the startup lock. Re-probe before any mutation.
        let readiness = self.backend.probe(&request.socket).await?;
        if let Some(ready) = validate_ready(&request, readiness.clone())? {
            return Ok(receipt(
                &request,
                DaemonActivation::AlreadyRunning,
                ready,
                started,
            ));
        }
        if matches!(readiness, DaemonReadiness::StaleSocket { .. }) {
            self.backend.recover_stale_socket(&request.socket).await?;
        }

        let activation = self.backend.activate(request.mode, &request.socket).await?;
        let deadline = started + request.startup_timeout;
        loop {
            let readiness = self.backend.probe(&request.socket).await?;
            if let Some(ready) = validate_ready(&request, readiness.clone())? {
                return Ok(receipt(&request, activation, ready, started));
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(DaemonLifecycleError::ReadinessTimeout {
                    timeout_ms: duration_ms(request.startup_timeout),
                    last_readiness: readiness,
                    diagnostic: self.backend.diagnose(&request.socket).await,
                });
            }
            tokio::time::sleep(request.poll_interval.min(deadline - now)).await;
        }
    }
}

fn validate_ready(
    request: &EnsureDaemonRequest,
    readiness: DaemonReadiness,
) -> Result<Option<(u16, String)>, DaemonLifecycleError> {
    let DaemonReadiness::Ready {
        protocol_version,
        runtime_version,
    } = readiness
    else {
        return Ok(None);
    };
    if protocol_version != request.expected_protocol_version {
        return Err(DaemonLifecycleError::ProtocolMismatch {
            expected: request.expected_protocol_version,
            actual: protocol_version,
        });
    }
    if let Some(expected) = request.expected_runtime_version.as_ref() {
        if &runtime_version != expected {
            return Err(DaemonLifecycleError::RuntimeVersionMismatch {
                expected: expected.clone(),
                actual: runtime_version,
            });
        }
    }
    Ok(Some((protocol_version, runtime_version)))
}

fn receipt(
    request: &EnsureDaemonRequest,
    activation: DaemonActivation,
    ready: (u16, String),
    started: Instant,
) -> DaemonReadyReceipt {
    DaemonReadyReceipt {
        mode: request.mode,
        activation,
        protocol_version: ready.0,
        runtime_version: ready.1,
        waited_ms: duration_ms(started.elapsed()),
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
