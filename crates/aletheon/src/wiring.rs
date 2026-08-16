//! Concrete Core/daemon wiring owned by the `aletheon` composition root.
//!
//! This module is intentionally the only binary-owned place that opens the
//! user runtime and binds lifecycle services. CLI and presentation code only
//! see the facade in [`crate::launcher`].

pub mod approval_service;
pub mod cognitive_runtime;
pub mod embodiment;
pub mod evolution_coordinator;
pub mod governed_review;
pub mod mode_router;
pub mod workspace_trust;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::launcher::{CoreLaunch, DaemonLaunch, EnsureUserDaemon, EnsureUserDaemonError};

pub mod adapters;
pub mod application;
pub(crate) mod composition;
pub mod core_rpc;
pub mod core_runtime;
pub mod daemon;
pub(crate) mod doctor;
pub(crate) mod domain;
pub mod exec;
pub(crate) mod exec_session;
pub(crate) mod extension;
pub mod readiness;
pub mod user_runtime;

fn select_daemon_socket(
    command: Option<PathBuf>,
    parent: Option<PathBuf>,
    environment: Option<PathBuf>,
    default: PathBuf,
) -> PathBuf {
    command.or(parent).or(environment).unwrap_or(default)
}

/// Load a dotenv file for the binary-owned daemon boundary.
///
/// Environment variables supplied by systemd or the invoking shell always
/// win; this is intentionally the same small, non-expanding parser used by
/// the legacy host during the staged cutover.
fn load_dotenv(path: &std::path::Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
}

pub async fn run_core(request: CoreLaunch) -> Result<()> {
    let runtime =
        core_runtime::MachineInferenceRuntime::bootstrap(request.config.as_deref(), request.socket)
            .await?;
    info!(
        socket = %runtime.socket_path().display(),
        providers = runtime.provider_count(),
        "System inference core started"
    );
    runtime.run().await
}

pub async fn run_daemon(request: DaemonLaunch) -> Result<()> {
    if let Some(env_path) = request.env.as_ref() {
        load_dotenv(env_path);
    }
    if let Some(container) = request.container.as_ref() {
        tracing::warn!(
            image = %request.image,
            container = %container,
            "container host selection is ignored by the per-user runtime boundary"
        );
    }
    let paths = ::contracts::paths::UserRuntimePaths::resolve(
        &::contracts::paths::ProcessRuntimeEnvironment,
    )?;
    paths.prepare()?;
    let _authority =
        readiness::acquire_daemon_authority(&paths.runtime_root.join("daemon-authority.lock"))?;
    let socket = select_daemon_socket(
        request.command_socket,
        request.parent_socket,
        std::env::var_os("ALETHEON_SOCKET")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        paths.socket_path(),
    );
    let config = user_runtime::UserRuntimeConfig::load(
        request.config.as_deref(),
        paths,
        socket,
        request.enable_evolution,
        request.enable_execd,
    )?;
    let core_socket = std::env::var_os("ALETHEON_CORE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock"));
    let inference = Arc::new(core_rpc::CoreRpcClient::new(core_socket));
    user_runtime::UserRuntime::bootstrap(config, inference)
        .await?
        .run()
        .await
}

pub async fn ensure_user_daemon(
    request: EnsureUserDaemon,
) -> Result<::application::daemon_lifecycle::DaemonReadyReceipt, EnsureUserDaemonError> {
    let paths = ::contracts::paths::UserRuntimePaths::resolve(
        &::contracts::paths::ProcessRuntimeEnvironment,
    )?;
    paths.prepare()?;
    let socket = request.socket.unwrap_or_else(|| paths.socket_path());
    let executable = std::env::current_exe().map_err(EnsureUserDaemonError::CurrentExecutable)?;
    let mode = readiness::detect_install_mode(&socket).await;
    let backend = Arc::new(readiness::ProcessDaemonLifecycleBackend::new(
        executable, mode,
    ));
    let startup_lock = Arc::new(readiness::FileStartupLock::new(
        paths.runtime_root.join("daemon-startup.lock"),
    ));
    ::application::daemon_lifecycle::DaemonLifecycleService::new(backend, startup_lock)
        .ensure_running(::application::daemon_lifecycle::EnsureDaemonRequest {
            socket,
            mode,
            startup_timeout: request.startup_timeout,
            poll_interval: Duration::from_millis(50),
            expected_protocol_version: ::contracts::CLIENT_PROTOCOL_VERSION,
            expected_runtime_version: Some(env!("CARGO_PKG_VERSION").into()),
        })
        .await
        .map_err(Into::into)
}
