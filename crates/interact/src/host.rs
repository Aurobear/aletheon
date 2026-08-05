//! Client host requests kept above the typed protocol and TUI implementation.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use fabric::paths::{ProcessRuntimeEnvironment, RuntimeEnvironment, UserRuntimePaths};

// A healthy cold start may restore durable cognition state before accepting the
// initialize handshake. Bootstrap failures are observed independently and are
// still projected immediately rather than waiting for this readiness deadline.
const CLIENT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DaemonStartupDiagnostic {
    pub schema_version: u16,
    pub code: &'static str,
    pub socket: String,
    pub timeout_ms: u64,
    pub detail: String,
    pub recovery: &'static str,
}

#[derive(Debug)]
pub struct DaemonStartupError {
    diagnostic: DaemonStartupDiagnostic,
    source: executive::host::launcher::EnsureUserDaemonError,
}

impl DaemonStartupError {
    pub fn diagnostic(&self) -> &DaemonStartupDiagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for DaemonStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "daemon startup failed: ")?;
        match serde_json::to_string(&self.diagnostic) {
            Ok(json) => formatter.write_str(&json),
            Err(_) => formatter.write_str(&self.diagnostic.detail),
        }
    }
}

impl Error for DaemonStartupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[async_trait::async_trait]
trait DaemonEnsurer: Send + Sync {
    async fn ensure(
        &self,
        request: executive::host::launcher::EnsureUserDaemon,
    ) -> Result<
        executive::application::daemon_lifecycle::DaemonReadyReceipt,
        executive::host::launcher::EnsureUserDaemonError,
    >;
}

struct ExecutiveDaemonEnsurer;

#[async_trait::async_trait]
impl DaemonEnsurer for ExecutiveDaemonEnsurer {
    async fn ensure(
        &self,
        request: executive::host::launcher::EnsureUserDaemon,
    ) -> Result<
        executive::application::daemon_lifecycle::DaemonReadyReceipt,
        executive::host::launcher::EnsureUserDaemonError,
    > {
        executive::host::launcher::ensure_user_daemon(request).await
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceLaunch {
    pub cwd: Option<PathBuf>,
    pub add_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLaunch {
    pub socket: Option<PathBuf>,
    pub workspace: WorkspaceLaunch,
    pub message: String,
    pub required_agent_runtimes: Vec<String>,
    pub task_kind: Option<fabric::TaskKind>,
    pub session_id: Option<fabric::SessionId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InitialSession {
    #[default]
    New,
    Resume(fabric::SessionId),
    Pick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiLaunch {
    pub socket: Option<PathBuf>,
    pub workspace: WorkspaceLaunch,
    pub required_agent_runtimes: Vec<String>,
    pub task_kind: Option<fabric::TaskKind>,
    pub initial_session: InitialSession,
}

fn agent_runtime_requirements(runtime_ids: Vec<String>) -> Vec<fabric::TurnRequirement> {
    runtime_ids
        .into_iter()
        .map(|runtime_id| fabric::TurnRequirement::InvokeAgentRuntime { runtime_id })
        .collect()
}

fn resolve_socket_with(
    explicit: Option<PathBuf>,
    environment: &impl RuntimeEnvironment,
) -> anyhow::Result<PathBuf> {
    if let Some(socket) = explicit {
        return Ok(socket);
    }
    if let Some(socket) = environment
        .var_os("ALETHEON_SOCKET")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Ok(socket);
    }
    Ok(UserRuntimePaths::resolve(environment)?.socket_path())
}

/// Resolve the current user's control socket using the canonical precedence:
/// explicit CLI value, `ALETHEON_SOCKET`, then XDG runtime.
pub(crate) fn resolve_user_socket(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    resolve_socket_with(explicit, &ProcessRuntimeEnvironment)
}

async fn ensure_resolved_user_socket(
    socket: &std::path::Path,
    ensurer: &dyn DaemonEnsurer,
) -> Result<(), DaemonStartupError> {
    ensurer
        .ensure(executive::host::launcher::EnsureUserDaemon {
            socket: Some(socket.to_path_buf()),
            startup_timeout: CLIENT_STARTUP_TIMEOUT,
        })
        .await
        .map(|_| ())
        .map_err(|source| DaemonStartupError {
            diagnostic: DaemonStartupDiagnostic {
                schema_version: 1,
                code: source.diagnostic_code(),
                socket: socket.to_string_lossy().into_owned(),
                timeout_ms: CLIENT_STARTUP_TIMEOUT.as_millis() as u64,
                detail: source.to_string(),
                recovery: "run `aletheon doctor --json` and inspect the daemon service",
            },
            source,
        })
}

fn resolve_workspace(selection: WorkspaceLaunch) -> anyhow::Result<fabric::WorkspacePolicy> {
    let process_cwd = std::env::current_dir()
        .map_err(|source| anyhow::anyhow!("cannot resolve process cwd: {source}"))?;
    let mode = permission_mode_from_environment();
    let profile = if mode.is_full() {
        fabric::PermissionProfileId::danger_full_access()
    } else {
        fabric::PermissionProfileId::workspace_write()
    };
    let mut add_dirs = selection.add_dirs;
    if mode.is_full()
        && !add_dirs
            .iter()
            .any(|path| path == std::path::Path::new("/"))
    {
        add_dirs.push(PathBuf::from("/"));
    }
    Ok(fabric::WorkspaceSelection::new(selection.cwd, add_dirs)
        .resolve_with_profile(&process_cwd, &profile)?)
}

pub(crate) fn permission_mode_from_environment() -> fabric::permission::HostPermissionMode {
    match std::env::var("ALETHEON_PERMISSION_MODE").as_deref() {
        Ok("full" | "unrestricted") => fabric::permission::HostPermissionMode::Full,
        Ok("dev" | "developer") => fabric::permission::HostPermissionMode::Developer,
        _ => fabric::permission::HostPermissionMode::Safe,
    }
}

pub async fn run_single_message(request: MessageLaunch) -> anyhow::Result<()> {
    let workspace = resolve_workspace(request.workspace)?;
    std::env::set_current_dir(workspace.cwd())?;
    let socket = resolve_user_socket(request.socket)?;
    ensure_resolved_user_socket(&socket, &ExecutiveDaemonEnsurer).await?;
    crate::single_message::run(
        &socket,
        &request.message,
        &workspace,
        agent_runtime_requirements(request.required_agent_runtimes),
        request.task_kind,
        request.session_id,
    )
    .await
}

pub async fn run_tui(request: TuiLaunch, config: crate::tui::TestConfig) -> anyhow::Result<()> {
    let workspace = resolve_workspace(request.workspace)?;
    std::env::set_current_dir(workspace.cwd())?;
    let socket = resolve_user_socket(request.socket)?;
    ensure_resolved_user_socket(&socket, &ExecutiveDaemonEnsurer).await?;
    crate::tui::run_with_workspace_requirements_and_task_kind(
        socket.to_string_lossy().as_ref(),
        config,
        workspace,
        agent_runtime_requirements(request.required_agent_runtimes),
        request.task_kind,
        request.initial_session,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::Path;

    use super::*;

    struct RecordingEnsurer {
        request: std::sync::Mutex<Option<executive::host::launcher::EnsureUserDaemon>>,
    }

    #[async_trait::async_trait]
    impl DaemonEnsurer for RecordingEnsurer {
        async fn ensure(
            &self,
            request: executive::host::launcher::EnsureUserDaemon,
        ) -> Result<
            executive::application::daemon_lifecycle::DaemonReadyReceipt,
            executive::host::launcher::EnsureUserDaemonError,
        > {
            *self.request.lock().unwrap() = Some(request);
            Ok(
                executive::application::daemon_lifecycle::DaemonReadyReceipt {
                    mode:
                        executive::application::daemon_lifecycle::DaemonInstallMode::SystemInstall,
                    activation:
                        executive::application::daemon_lifecycle::DaemonActivation::ActivatedService,
                    protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
                    runtime_version: env!("CARGO_PKG_VERSION").into(),
                    waited_ms: 4,
                },
            )
        }
    }

    struct FailingEnsurer;

    #[async_trait::async_trait]
    impl DaemonEnsurer for FailingEnsurer {
        async fn ensure(
            &self,
            _request: executive::host::launcher::EnsureUserDaemon,
        ) -> Result<
            executive::application::daemon_lifecycle::DaemonReadyReceipt,
            executive::host::launcher::EnsureUserDaemonError,
        > {
            Err(executive::host::launcher::EnsureUserDaemonError::Lifecycle(
                executive::application::daemon_lifecycle::DaemonLifecycleError::ReadinessTimeout {
                    timeout_ms: 30_000,
                    last_readiness:
                        executive::application::daemon_lifecycle::DaemonReadiness::Absent,
                    diagnostic: "service configuration rejected startup".into(),
                },
            ))
        }
    }

    #[derive(Default)]
    struct FakeEnvironment(BTreeMap<String, OsString>);

    impl RuntimeEnvironment for FakeEnvironment {
        fn var_os(&self, key: &str) -> Option<OsString> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn endpoint_precedence_is_explicit_then_environment_then_xdg() {
        let explicit = resolve_socket_with(
            Some(PathBuf::from("/tmp/explicit.sock")),
            &FakeEnvironment::default(),
        )
        .unwrap();
        assert_eq!(explicit, Path::new("/tmp/explicit.sock"));

        let environment = FakeEnvironment(BTreeMap::from([
            ("ALETHEON_SOCKET".into(), "/tmp/environment.sock".into()),
            ("XDG_RUNTIME_DIR".into(), "/run/user/1001".into()),
            ("HOME".into(), "/home/a".into()),
        ]));
        assert_eq!(
            resolve_socket_with(None, &environment).unwrap(),
            Path::new("/tmp/environment.sock")
        );

        let xdg = FakeEnvironment(BTreeMap::from([
            ("XDG_RUNTIME_DIR".into(), "/run/user/1001".into()),
            ("HOME".into(), "/home/a".into()),
        ]));
        assert_eq!(
            resolve_socket_with(None, &xdg).unwrap(),
            Path::new("/run/user/1001/aletheon/aletheon.sock")
        );
    }

    #[test]
    fn runtime_ids_become_typed_turn_requirements() {
        assert_eq!(
            agent_runtime_requirements(vec!["pi-rpc".into(), "native-cognit".into()]),
            vec![
                fabric::TurnRequirement::InvokeAgentRuntime {
                    runtime_id: "pi-rpc".into(),
                },
                fabric::TurnRequirement::InvokeAgentRuntime {
                    runtime_id: "native-cognit".into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn u_boot_001_absent_socket_runs_ensure_before_client_connection() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("not-started.sock");
        assert!(!socket.exists());
        let ensurer = RecordingEnsurer {
            request: std::sync::Mutex::new(None),
        };

        ensure_resolved_user_socket(&socket, &ensurer)
            .await
            .unwrap();

        let request = ensurer.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.socket.as_deref(), Some(socket.as_path()));
        assert_eq!(request.startup_timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn u_boot_002_startup_failure_has_bounded_structured_diagnostic() {
        let socket = std::path::Path::new("/tmp/config-error.sock");
        let error = ensure_resolved_user_socket(socket, &FailingEnsurer)
            .await
            .unwrap_err();

        assert_eq!(error.diagnostic().schema_version, 1);
        assert_eq!(error.diagnostic().code, "daemon_readiness_timeout");
        assert_eq!(error.diagnostic().socket, "/tmp/config-error.sock");
        assert_eq!(error.diagnostic().timeout_ms, 30_000);
        assert!(error
            .diagnostic()
            .detail
            .contains("service configuration rejected startup"));
        assert_eq!(
            serde_json::to_value(error.diagnostic()).unwrap()["code"],
            "daemon_readiness_timeout"
        );
    }
}
