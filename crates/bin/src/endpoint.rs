//! Client endpoint resolution for the per-user runtime.

use std::path::PathBuf;

use fabric::paths::{RuntimeEnvironment, UserPathError, UserRuntimePaths};

/// Resolve a socket from already-materialized sources.
///
/// This pure helper documents the precedence independently of environment and
/// filesystem access: command-line value, environment value, then the private
/// per-user runtime socket.
pub fn resolve_socket(
    explicit: Option<PathBuf>,
    environment: Option<PathBuf>,
    paths: &UserRuntimePaths,
) -> PathBuf {
    explicit
        .or(environment)
        .unwrap_or_else(|| paths.socket_path())
}

/// Resolve the endpoint used by a local client.
///
/// Explicit and environment-provided sockets return before XDG path
/// resolution. This keeps an explicit endpoint usable in recovery and
/// administration environments that do not define `XDG_RUNTIME_DIR`.
pub fn resolve_client_socket(
    explicit: Option<PathBuf>,
    env: &impl RuntimeEnvironment,
) -> Result<PathBuf, UserPathError> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = env
        .var_os("ALETHEON_SOCKET")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Ok(path);
    }
    let paths = UserRuntimePaths::resolve(env)?;
    Ok(paths.socket_path())
}

/// Preserve the daemon subcommand's socket override: its value wins over the
/// parent CLI value before normal environment/XDG endpoint resolution.
pub fn resolve_daemon_socket(
    command_socket: Option<PathBuf>,
    parent_socket: Option<PathBuf>,
    env: &impl RuntimeEnvironment,
) -> Result<PathBuf, UserPathError> {
    resolve_client_socket(command_socket.or(parent_socket), env)
}
