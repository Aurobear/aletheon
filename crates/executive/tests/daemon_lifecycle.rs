use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use executive::application::daemon_lifecycle::{
    resolve_install_mode, DaemonActivation, DaemonInstallMode, DaemonLifecycleBackend,
    DaemonLifecycleError, DaemonLifecycleService, DaemonReadiness, EnsureDaemonRequest,
    InstallModeFacts, StartupLease, StartupLockPort,
};

struct TestLease {
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl StartupLease for TestLease {}

#[derive(Default)]
struct TestStartupLock {
    lock: Arc<tokio::sync::Mutex<()>>,
}

#[async_trait::async_trait]
impl StartupLockPort for TestStartupLock {
    async fn acquire(
        &self,
        timeout: Duration,
    ) -> Result<Box<dyn StartupLease>, DaemonLifecycleError> {
        let guard = tokio::time::timeout(timeout, self.lock.clone().lock_owned())
            .await
            .map_err(|_| DaemonLifecycleError::LockTimeout {
                timeout_ms: timeout.as_millis() as u64,
            })?;
        Ok(Box::new(TestLease { _guard: guard }))
    }
}

struct FakeBackend {
    stale: AtomicBool,
    unready: bool,
    ready: AtomicBool,
    activations: AtomicUsize,
    recoveries: AtomicUsize,
    protocol_version: u16,
    runtime_version: &'static str,
    become_ready: bool,
}

struct SlowProbeBackend;

struct FailedBootstrapBackend;

#[async_trait::async_trait]
impl DaemonLifecycleBackend for FailedBootstrapBackend {
    async fn probe(&self, _socket: &Path) -> Result<DaemonReadiness, DaemonLifecycleError> {
        Ok(DaemonReadiness::Failed {
            detail: "configuration rejected before runtime bootstrap".into(),
        })
    }

    async fn recover_stale_socket(&self, _socket: &Path) -> Result<(), DaemonLifecycleError> {
        unreachable!("a failed bootstrap must not mutate the endpoint")
    }

    async fn activate(
        &self,
        _mode: DaemonInstallMode,
        _socket: &Path,
    ) -> Result<DaemonActivation, DaemonLifecycleError> {
        unreachable!("a failed bootstrap must not be activated again")
    }

    async fn diagnose(&self, _socket: &Path) -> String {
        unreachable!("a typed bootstrap failure does not need timeout diagnostics")
    }
}

#[async_trait::async_trait]
impl DaemonLifecycleBackend for SlowProbeBackend {
    async fn probe(&self, _socket: &Path) -> Result<DaemonReadiness, DaemonLifecycleError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(DaemonReadiness::Absent)
    }

    async fn recover_stale_socket(&self, _socket: &Path) -> Result<(), DaemonLifecycleError> {
        unreachable!("a timed-out probe must not mutate the endpoint")
    }

    async fn activate(
        &self,
        _mode: DaemonInstallMode,
        _socket: &Path,
    ) -> Result<DaemonActivation, DaemonLifecycleError> {
        unreachable!("a timed-out probe must not activate a daemon")
    }

    async fn diagnose(&self, _socket: &Path) -> String {
        "probe deadline exhausted".into()
    }
}

impl FakeBackend {
    fn stale_then_ready() -> Self {
        Self {
            stale: AtomicBool::new(true),
            unready: false,
            ready: AtomicBool::new(false),
            activations: AtomicUsize::new(0),
            recoveries: AtomicUsize::new(0),
            protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
            runtime_version: env!("CARGO_PKG_VERSION"),
            become_ready: true,
        }
    }
}

#[async_trait::async_trait]
impl DaemonLifecycleBackend for FakeBackend {
    async fn probe(&self, _socket: &Path) -> Result<DaemonReadiness, DaemonLifecycleError> {
        if self.ready.load(Ordering::SeqCst) {
            return Ok(DaemonReadiness::Ready {
                protocol_version: self.protocol_version,
                runtime_version: self.runtime_version.into(),
            });
        }
        if self.stale.load(Ordering::SeqCst) {
            return Ok(DaemonReadiness::StaleSocket {
                detail: "connection refused".into(),
            });
        }
        if self.unready {
            return Ok(DaemonReadiness::Unready {
                detail: "accepted connections without handshake".into(),
            });
        }
        Ok(DaemonReadiness::Absent)
    }

    async fn recover_stale_socket(&self, _socket: &Path) -> Result<(), DaemonLifecycleError> {
        self.recoveries.fetch_add(1, Ordering::SeqCst);
        self.stale.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn activate(
        &self,
        mode: DaemonInstallMode,
        _socket: &Path,
    ) -> Result<DaemonActivation, DaemonLifecycleError> {
        self.activations.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        if self.become_ready {
            self.ready.store(true, Ordering::SeqCst);
        }
        Ok(match mode {
            DaemonInstallMode::DevForeground => DaemonActivation::SpawnedForeground,
            DaemonInstallMode::SystemInstall | DaemonInstallMode::UserLocal => {
                DaemonActivation::ActivatedService
            }
        })
    }

    async fn diagnose(&self, _socket: &Path) -> String {
        "fake daemon remained unready".into()
    }
}

fn request() -> EnsureDaemonRequest {
    EnsureDaemonRequest {
        socket: "/tmp/aletheon-lifecycle-test.sock".into(),
        mode: DaemonInstallMode::SystemInstall,
        startup_timeout: Duration::from_secs(1),
        poll_interval: Duration::from_millis(1),
        expected_protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
        expected_runtime_version: Some(env!("CARGO_PKG_VERSION").into()),
    }
}

#[test]
fn install_mode_resolution_is_deterministic() {
    assert_eq!(
        resolve_install_mode(InstallModeFacts {
            system_install_available: true,
            user_local_install_available: true,
        }),
        DaemonInstallMode::SystemInstall
    );
    assert_eq!(
        resolve_install_mode(InstallModeFacts {
            system_install_available: false,
            user_local_install_available: true,
        }),
        DaemonInstallMode::UserLocal
    );
    assert_eq!(
        resolve_install_mode(InstallModeFacts::default()),
        DaemonInstallMode::DevForeground
    );
}

#[tokio::test]
async fn u_boot_003_concurrent_clients_activate_one_authority_and_recover_stale_socket() {
    let backend = Arc::new(FakeBackend::stale_then_ready());
    let startup_lock = Arc::new(TestStartupLock::default());
    let first = DaemonLifecycleService::new(backend.clone(), startup_lock.clone());
    let second = DaemonLifecycleService::new(backend.clone(), startup_lock);

    let (first, second) = tokio::join!(
        first.ensure_running(request()),
        second.ensure_running(request())
    );
    let receipts = [first.unwrap(), second.unwrap()];

    assert_eq!(backend.activations.load(Ordering::SeqCst), 1);
    assert_eq!(backend.recoveries.load(Ordering::SeqCst), 1);
    assert!(receipts
        .iter()
        .any(|receipt| receipt.activation == DaemonActivation::ActivatedService));
    assert!(receipts
        .iter()
        .any(|receipt| receipt.activation == DaemonActivation::AlreadyRunning));
}

#[tokio::test]
async fn incompatible_runtime_version_fails_closed() {
    let backend = Arc::new(FakeBackend {
        stale: AtomicBool::new(false),
        unready: false,
        ready: AtomicBool::new(true),
        activations: AtomicUsize::new(0),
        recoveries: AtomicUsize::new(0),
        protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
        runtime_version: "old-runtime",
        become_ready: true,
    });
    let service = DaemonLifecycleService::new(backend, Arc::new(TestStartupLock::default()));

    assert!(matches!(
        service.ensure_running(request()).await,
        Err(DaemonLifecycleError::RuntimeVersionMismatch { .. })
    ));
}

#[tokio::test]
async fn readiness_timeout_carries_typed_last_state_and_doctor_detail() {
    let backend = Arc::new(FakeBackend {
        stale: AtomicBool::new(false),
        unready: false,
        ready: AtomicBool::new(false),
        activations: AtomicUsize::new(0),
        recoveries: AtomicUsize::new(0),
        protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
        runtime_version: env!("CARGO_PKG_VERSION"),
        become_ready: false,
    });
    let service = DaemonLifecycleService::new(backend, Arc::new(TestStartupLock::default()));
    let mut request = request();
    request.startup_timeout = Duration::from_millis(20);

    let Err(DaemonLifecycleError::ReadinessTimeout {
        last_readiness,
        diagnostic,
        ..
    }) = service.ensure_running(request).await
    else {
        panic!("expected a typed readiness timeout");
    };
    assert!(matches!(
        last_readiness,
        DaemonReadiness::Absent | DaemonReadiness::Unready { .. }
    ));
    assert_eq!(diagnostic, "fake daemon remained unready");
}

#[tokio::test]
async fn unresponsive_existing_authority_is_not_replaced() {
    let backend = Arc::new(FakeBackend {
        stale: AtomicBool::new(false),
        unready: true,
        ready: AtomicBool::new(false),
        activations: AtomicUsize::new(0),
        recoveries: AtomicUsize::new(0),
        protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
        runtime_version: env!("CARGO_PKG_VERSION"),
        become_ready: false,
    });
    let service =
        DaemonLifecycleService::new(backend.clone(), Arc::new(TestStartupLock::default()));
    let mut request = request();
    request.startup_timeout = Duration::from_millis(20);

    assert!(matches!(
        service.ensure_running(request).await,
        Err(DaemonLifecycleError::ReadinessTimeout {
            last_readiness: DaemonReadiness::Unready { .. },
            ..
        })
    ));
    assert_eq!(backend.activations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn readiness_probe_cannot_overrun_the_monotonic_startup_deadline() {
    let service = DaemonLifecycleService::new(
        Arc::new(SlowProbeBackend),
        Arc::new(TestStartupLock::default()),
    );
    let mut request = request();
    request.startup_timeout = Duration::from_millis(20);
    let started = std::time::Instant::now();

    assert!(matches!(
        service.ensure_running(request).await,
        Err(DaemonLifecycleError::ReadinessTimeout {
            last_readiness: DaemonReadiness::Unready { detail },
            diagnostic,
            ..
        }) if detail.contains("remaining startup deadline")
            && diagnostic == "probe deadline exhausted"
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn u_boot_002_bootstrap_failure_returns_without_waiting_for_readiness_deadline() {
    let service = DaemonLifecycleService::new(
        Arc::new(FailedBootstrapBackend),
        Arc::new(TestStartupLock::default()),
    );
    let mut request = request();
    request.startup_timeout = Duration::from_secs(30);
    let started = std::time::Instant::now();

    assert!(matches!(
        service.ensure_running(request).await,
        Err(DaemonLifecycleError::BootstrapFailed(detail))
            if detail.contains("configuration rejected")
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
}
