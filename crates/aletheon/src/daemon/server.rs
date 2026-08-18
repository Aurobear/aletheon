use std::ffi::OsString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use anyhow::Result;
use gateway::protocol::{
    ApprovalRequestedEvent, Event as TypedGatewayEvent, RuntimeProgressEvent, SessionRef, TurnRef,
};
use tokio::net::UnixListener;
use tracing::error;

/// Translate compatibility notifications into distinct typed Gateway event
/// schemas for a typed connection. Progress is presentation evidence;
/// Session/Turn terminal authority still comes from projection/settlement.
pub(crate) fn typed_gateway_notification(message: &str) -> Option<TypedGatewayEvent> {
    let value: serde_json::Value = serde_json::from_str(message).ok()?;
    let method = value.get("method")?.as_str()?;
    let (kind, payload) = match method {
        "event" => ("client_event", value.get("params")?.clone()),
        "approval_request" => {
            let params = value.get("params")?.clone();
            let approval = ApprovalRequestedEvent {
                session: SessionRef(params.get("session")?.as_str()?.to_owned()),
                turn: TurnRef(params.get("turn")?.as_str()?.to_owned()),
                choice_id: params.get("approval_id")?.as_str()?.to_owned(),
                tool: params.get("tool")?.as_str()?.to_owned(),
                action_summary: params.get("action_summary")?.as_str()?.to_owned(),
                risk_level: params.get("risk_level")?.as_str()?.to_owned(),
                detail: params
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                scope_subject: params
                    .get("scope_subject")
                    .cloned()
                    .filter(|value| !value.is_null())
                    .map(serde_json::from_value)
                    .transpose()
                    .ok()?,
            };
            return Some(TypedGatewayEvent::ApprovalRequested(approval));
        }
        _ => return None,
    };
    Some(TypedGatewayEvent::Progress(RuntimeProgressEvent {
        kind: kind.into(),
        payload,
    }))
}

/// Filesystem visibility of a path-bound daemon socket.
pub trait ActivationEnvironment: ::contracts::paths::RuntimeEnvironment {
    fn remove_var(&self, key: &str);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessActivationEnvironment;

impl ::contracts::paths::RuntimeEnvironment for ProcessActivationEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

impl ActivationEnvironment for ProcessActivationEnvironment {
    fn remove_var(&self, key: &str) {
        std::env::remove_var(key);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("LISTEN_PID and LISTEN_FDS must either both be set or both be absent")]
    IncompleteEnvironment,
    #[error("LISTEN_PID is invalid")]
    InvalidPid,
    #[error("LISTEN_PID {actual} does not match current pid {expected}")]
    WrongPid { expected: u32, actual: u32 },
    #[error("LISTEN_FDS must be exactly 1, got {0}")]
    InvalidFdCount(u32),
    #[error("socket activation declared one listener but no inherited fd was supplied")]
    MissingFd,
    #[error("inherited fd is not an AF_UNIX stream listener")]
    InvalidListener,
    #[error("unable to inspect inherited listener: {0}")]
    Inspection(#[source] std::io::Error),
}

/// Validate and adopt a duplicated systemd activation descriptor.
///
/// The caller supplies an owned duplicate instead of this function taking fd 3
/// directly. That keeps descriptor ownership explicit and makes tests safe to
/// run in-process without replacing a real fd 3.
pub fn inherited_listener(
    env: &impl ActivationEnvironment,
    inherited_fd: Option<OwnedFd>,
) -> Result<Option<UnixListener>, ActivationError> {
    if !activation_is_declared(env)? {
        return Ok(None);
    }

    let inherited_fd = inherited_fd.ok_or(ActivationError::MissingFd)?;
    validate_unix_stream_listener(&inherited_fd)?;
    let listener: std::os::unix::net::UnixListener = inherited_fd.into();
    listener
        .set_nonblocking(true)
        .map_err(ActivationError::Inspection)?;
    let listener = UnixListener::from_std(listener).map_err(ActivationError::Inspection)?;
    env.remove_var("LISTEN_PID");
    env.remove_var("LISTEN_FDS");
    Ok(Some(listener))
}

/// Adopt the single listener passed by systemd in production.
///
/// The inherited descriptor is duplicated before conversion so descriptor 3
/// remains owned by the process activation contract rather than by a testable
/// helper. Absence of activation is not an error and lets the caller bind the
/// configured path instead.
pub fn process_inherited_listener() -> Result<Option<UnixListener>, ActivationError> {
    let env = ProcessActivationEnvironment;
    if !activation_is_declared(&env)? {
        return Ok(None);
    }
    // SAFETY: `fcntl` does not take ownership of fd 3. On success it returns a
    // new close-on-exec descriptor owned by this function.
    let duplicate = unsafe { libc::fcntl(3, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate == -1 {
        return Err(ActivationError::Inspection(std::io::Error::last_os_error()));
    }
    // SAFETY: a successful F_DUPFD_CLOEXEC returns a fresh owned descriptor.
    let duplicate = unsafe { OwnedFd::from_raw_fd(duplicate) };
    inherited_listener(&env, Some(duplicate))
}

fn activation_is_declared(env: &impl ActivationEnvironment) -> Result<bool, ActivationError> {
    let listen_pid = env.var_os("LISTEN_PID");
    let listen_fds = env.var_os("LISTEN_FDS");
    let (listen_pid, listen_fds) = match (listen_pid, listen_fds) {
        (None, None) => return Ok(false),
        (Some(pid), Some(fds)) => (pid, fds),
        _ => return Err(ActivationError::IncompleteEnvironment),
    };

    let listen_pid = listen_pid
        .to_str()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(ActivationError::InvalidPid)?;
    let current_pid = std::process::id();
    if listen_pid != current_pid {
        return Err(ActivationError::WrongPid {
            expected: current_pid,
            actual: listen_pid,
        });
    }
    let listen_fds = listen_fds
        .to_str()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(ActivationError::InvalidFdCount(0))?;
    if listen_fds != 1 {
        return Err(ActivationError::InvalidFdCount(listen_fds));
    }
    Ok(true)
}

fn validate_unix_stream_listener(fd: &OwnedFd) -> Result<(), ActivationError> {
    let socket_type = socket_option(fd, libc::SO_TYPE)?;
    let accepting = socket_option(fd, libc::SO_ACCEPTCONN)?;
    let family = socket_family(fd)?;
    if socket_type != libc::SOCK_STREAM || accepting != 1 || family != libc::AF_UNIX {
        return Err(ActivationError::InvalidListener);
    }
    Ok(())
}

fn socket_option(fd: &OwnedFd, option: libc::c_int) -> Result<libc::c_int, ActivationError> {
    let mut value: libc::c_int = 0;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `value` and `length` point to initialized writable storage, and
    // `fd` remains owned for the duration of the call.
    let result = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result == -1 {
        return Err(ActivationError::Inspection(std::io::Error::last_os_error()));
    }
    Ok(value)
}

fn socket_family(fd: &OwnedFd) -> Result<libc::c_int, ActivationError> {
    // SAFETY: zero is a valid initial byte representation for sockaddr_storage.
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: `address` and `length` describe valid writable storage, and `fd`
    // remains owned for the duration of the call.
    let result = unsafe {
        libc::getsockname(
            fd.as_raw_fd(),
            (&mut address as *mut libc::sockaddr_storage).cast(),
            &mut length,
        )
    };
    if result == -1 {
        return Err(ActivationError::Inspection(std::io::Error::last_os_error()));
    }
    Ok(libc::c_int::from(address.ss_family))
}

pub use crate::host::unix_server::{ConnectionContext, ConnectionRole};

pub use crate::host::unix_server::{SocketPrivacy, UnixServer};
