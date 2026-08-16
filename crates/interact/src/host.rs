//! Client host requests kept above the typed protocol and TUI implementation.

use std::path::PathBuf;

use ::contracts::paths::{ProcessRuntimeEnvironment, RuntimeEnvironment, UserRuntimePaths};

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
    pub task_kind: Option<::contracts::TaskKind>,
    pub session_id: Option<::contracts::SessionId>,
    pub requested_permission: gateway::protocol::RequestedPermissionMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InitialSession {
    #[default]
    New,
    Resume(::contracts::SessionId),
    Pick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiLaunch {
    pub socket: Option<PathBuf>,
    pub workspace: WorkspaceLaunch,
    pub required_agent_runtimes: Vec<String>,
    pub task_kind: Option<::contracts::TaskKind>,
    pub initial_session: InitialSession,
    pub requested_permission: gateway::protocol::RequestedPermissionMode,
}

fn agent_runtime_requirements(runtime_ids: Vec<String>) -> Vec<::contracts::TurnRequirement> {
    runtime_ids
        .into_iter()
        .map(|runtime_id| ::contracts::TurnRequirement::InvokeAgentRuntime { runtime_id })
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
pub fn resolve_user_socket(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    resolve_socket_with(explicit, &ProcessRuntimeEnvironment)
}

fn resolve_workspace(selection: WorkspaceLaunch) -> anyhow::Result<::contracts::WorkspacePolicy> {
    let process_cwd = std::env::current_dir()
        .map_err(|source| anyhow::anyhow!("cannot resolve process cwd: {source}"))?;
    // Workspace resolution is a requested client boundary. The authenticated
    // Gateway/Host computes effective permission; the TUI/CLI must not inspect
    // a local environment variable and claim an effective profile.
    Ok(
        ::contracts::WorkspaceSelection::new(selection.cwd, selection.add_dirs)
            .resolve_with_profile(
                &process_cwd,
                &::contracts::PermissionProfileId::workspace_write(),
            )?,
    )
}

pub async fn run_single_message(request: MessageLaunch) -> anyhow::Result<()> {
    let workspace = resolve_workspace(request.workspace)?;
    std::env::set_current_dir(workspace.cwd())?;
    let socket = resolve_user_socket(request.socket)?;
    crate::single_message::run(
        &socket,
        &request.message,
        &workspace,
        agent_runtime_requirements(request.required_agent_runtimes),
        request.task_kind,
        request.session_id,
        request.requested_permission,
    )
    .await
}

pub async fn run_tui(request: TuiLaunch, config: crate::tui::TestConfig) -> anyhow::Result<()> {
    let workspace = resolve_workspace(request.workspace)?;
    std::env::set_current_dir(workspace.cwd())?;
    let socket = resolve_user_socket(request.socket)?;
    crate::tui::run_with_workspace_requirements_and_task_kind(
        socket.to_string_lossy().as_ref(),
        config,
        workspace,
        agent_runtime_requirements(request.required_agent_runtimes),
        request.task_kind,
        request.initial_session,
        request.requested_permission,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::Path;

    use super::*;

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
    fn u_boot_001_client_resolution_does_not_construct_daemon_authority() {
        let socket = resolve_user_socket(Some(PathBuf::from("/tmp/official.sock"))).unwrap();
        assert_eq!(socket, Path::new("/tmp/official.sock"));
    }

    #[test]
    fn u_boot_002_socket_resolution_has_stable_client_precedence() {
        let environment = FakeEnvironment(BTreeMap::from([(
            "ALETHEON_SOCKET".into(),
            "/tmp/environment.sock".into(),
        )]));
        assert_eq!(
            resolve_socket_with(None, &environment).unwrap(),
            Path::new("/tmp/environment.sock")
        );
        assert_eq!(
            resolve_socket_with(Some(PathBuf::from("/tmp/explicit.sock")), &environment,).unwrap(),
            Path::new("/tmp/explicit.sock")
        );
    }

    #[test]
    fn runtime_ids_become_typed_turn_requirements() {
        assert_eq!(
            agent_runtime_requirements(vec!["pi-rpc".into(), "native-cognit".into()]),
            vec![
                ::contracts::TurnRequirement::InvokeAgentRuntime {
                    runtime_id: "pi-rpc".into(),
                },
                ::contracts::TurnRequirement::InvokeAgentRuntime {
                    runtime_id: "native-cognit".into(),
                },
            ]
        );
    }
}
