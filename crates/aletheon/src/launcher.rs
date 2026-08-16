//! Binary-owned launch boundary.
//!
//! The CLI enters Core, Daemon, Exec, and user-daemon lifecycle through this
//! module. Keeping the facade here prevents CLI parsing from constructing
//! domain or Runtime components itself.

pub use crate::wiring::exec::{
    ExecEventWriter, ExecHostOutcome, ExecLaunch, JsonlExecEventWriter, WorkspaceLaunch,
};

#[derive(Debug, Clone)]
pub struct CoreLaunch {
    pub config: Option<std::path::PathBuf>,
    pub socket: std::path::PathBuf,
}

#[derive(Debug, Clone)]
pub struct DaemonLaunch {
    pub config: Option<std::path::PathBuf>,
    pub env: Option<std::path::PathBuf>,
    pub command_socket: Option<std::path::PathBuf>,
    pub parent_socket: Option<std::path::PathBuf>,
    pub container: Option<String>,
    pub image: String,
    pub enable_evolution: bool,
    pub enable_execd: bool,
}

#[derive(Debug, Clone)]
pub struct EnsureUserDaemon {
    pub socket: Option<std::path::PathBuf>,
    pub startup_timeout: std::time::Duration,
}

impl Default for EnsureUserDaemon {
    fn default() -> Self {
        Self {
            socket: None,
            startup_timeout: std::time::Duration::from_secs(30),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnsureUserDaemonError {
    #[error(transparent)]
    Paths(#[from] ::contracts::paths::UserPathError),
    #[error("cannot resolve the current Aletheon executable: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error(transparent)]
    Lifecycle(#[from] ::application::daemon_lifecycle::DaemonLifecycleError),
}

pub async fn run_core(request: CoreLaunch) -> anyhow::Result<()> {
    crate::wiring::run_core(request).await
}

pub async fn run_daemon(request: DaemonLaunch) -> anyhow::Result<()> {
    crate::wiring::run_daemon(request).await
}

pub async fn ensure_user_daemon(
    request: EnsureUserDaemon,
) -> Result<::application::daemon_lifecycle::DaemonReadyReceipt, EnsureUserDaemonError> {
    crate::wiring::ensure_user_daemon(request).await
}

pub async fn run_exec(request: ExecLaunch) -> anyhow::Result<ExecHostOutcome> {
    crate::wiring::exec::run_exec(request).await
}

pub async fn run_exec_streaming(
    request: ExecLaunch,
    writer: std::sync::Arc<dyn ExecEventWriter>,
) -> anyhow::Result<ExecHostOutcome> {
    crate::wiring::exec::run_exec_streaming(request, writer).await
}
