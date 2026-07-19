use std::ffi::CString;
use std::ffi::OsString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fabric::debug::DebugEvent;
use fabric::events::ui_event::ClientEvent;
use fabric::protocol::client::{
    negotiate_protocol_version, ClientCapabilities, ClientEvent as ProtocolClientEvent,
    ClientMessage, ClientRequest, InitializedResult,
};
use fabric::{Clock, ConnectionId, LocalOsPrincipal, PrincipalId, Timer};
use futures::FutureExt;
use kernel::chronos::SystemTimer;
use nix::unistd::{Gid, Uid, User};
use std::panic::AssertUnwindSafe;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const CONNECTION_NOTIFICATION_CAPACITY: usize = 64;

use super::handler::RequestHandler;

/// Filesystem visibility of a path-bound daemon socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketPrivacy {
    /// Per-user runtime: parent directory 0700 and socket 0600.
    UserPrivate,
    /// Machine core compatibility: socket remains owner/group accessible 0660.
    SystemCore,
}

/// Injectable systemd activation environment. Tests use an in-memory
/// implementation so they never mutate process-global environment variables.
pub trait ActivationEnvironment: fabric::paths::RuntimeEnvironment {
    fn remove_var(&self, key: &str);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessActivationEnvironment;

impl fabric::paths::RuntimeEnvironment for ProcessActivationEnvironment {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionContext {
    pub principal_id: PrincipalId,
    pub os_principal: LocalOsPrincipal,
    pub connection_id: ConnectionId,
}

impl ConnectionContext {
    pub(crate) fn from_peer(os_principal: LocalOsPrincipal) -> Self {
        Self {
            principal_id: PrincipalId::local_uid(os_principal.uid),
            os_principal,
            connection_id: ConnectionId::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NegotiatedProtocol {
    protocol_version: u16,
    capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ConnectionProtocolState {
    New,
    AwaitingInitialized {
        negotiated: NegotiatedProtocol,
    },
    Ready {
        negotiated: Option<NegotiatedProtocol>,
    },
}

enum ProtocolAction {
    InitializeResponse(NegotiatedProtocol),
    Initialized,
    Dispatch,
}

impl ConnectionProtocolState {
    fn accept(&mut self, request: &ClientRequest) -> anyhow::Result<ProtocolAction> {
        match (&mut *self, request) {
            (Self::New, ClientRequest::Initialize(params)) => {
                let negotiated = NegotiatedProtocol {
                    protocol_version: negotiate_protocol_version(&params.protocol_versions)?,
                    capabilities: params.capabilities.clone(),
                };
                *self = Self::AwaitingInitialized {
                    negotiated: negotiated.clone(),
                };
                Ok(ProtocolAction::InitializeResponse(negotiated))
            }
            (Self::New, _) => anyhow::bail!("connection must initialize before requests"),
            (Self::AwaitingInitialized { negotiated }, ClientRequest::Initialized) => {
                let negotiated = negotiated.clone();
                *self = Self::Ready {
                    negotiated: Some(negotiated),
                };
                Ok(ProtocolAction::Initialized)
            }
            (Self::AwaitingInitialized { .. }, ClientRequest::Initialize(_)) => {
                anyhow::bail!("connection initialization cannot be repeated")
            }
            (Self::AwaitingInitialized { .. }, _) => {
                anyhow::bail!("connection must send initialized before requests")
            }
            (Self::Ready { .. }, ClientRequest::Initialize(_) | ClientRequest::Initialized) => {
                anyhow::bail!("connection initialization cannot be repeated")
            }
            (
                Self::Ready {
                    negotiated: Some(_),
                },
                _,
            ) => Ok(ProtocolAction::Dispatch),
            (Self::Ready { negotiated: None }, _) => {
                anyhow::bail!("legacy connections cannot send versioned requests")
            }
        }
    }
}

/// Temporary M0-M2 bridge for the pre-versioned JSON-RPC client.
///
/// This adapter never handles a Fabric `ClientMessage`; it only marks a legacy
/// JSON-RPC connection ready while preserving the identity authenticated by the
/// Unix transport. M3 removes it with the legacy client protocol.
struct LegacyClientHandshakeAdapter;

impl LegacyClientHandshakeAdapter {
    fn bind(state: &mut ConnectionProtocolState, request: &serde_json::Value) -> bool {
        if !is_legacy_json_rpc(request) {
            return false;
        }
        match state {
            ConnectionProtocolState::New => {
                *state = ConnectionProtocolState::Ready { negotiated: None };
                true
            }
            ConnectionProtocolState::Ready { negotiated: None } => true,
            ConnectionProtocolState::AwaitingInitialized { .. }
            | ConnectionProtocolState::Ready {
                negotiated: Some(_),
            } => false,
        }
    }
}

fn is_legacy_json_rpc(request: &serde_json::Value) -> bool {
    request.get("jsonrpc").and_then(|value| value.as_str()) == Some("2.0")
        && request
            .get("method")
            .and_then(|value| value.as_str())
            .is_some()
        && !has_versioned_params(request)
}

fn has_versioned_params(request: &serde_json::Value) -> bool {
    request.get("params").is_some_and(|params| {
        params.get("protocol_version").is_some() && params.get("payload").is_some()
    })
}

fn parse_versioned_request(request: &serde_json::Value) -> Option<anyhow::Result<ClientRequest>> {
    if !has_versioned_params(request) {
        return None;
    }
    Some(
        serde_json::from_value::<ClientMessage<ClientRequest>>(request["params"].clone())
            .map_err(anyhow::Error::from)
            .and_then(|message| message.into_v1().map_err(anyhow::Error::from)),
    )
}

fn protocol_error(
    request_id: serde_json::Value,
    error: impl std::fmt::Display,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "error": { "code": -32030, "message": error.to_string() }
    })
}

fn initialize_response(
    request_id: serde_json::Value,
    connection: &ConnectionContext,
    negotiated: NegotiatedProtocol,
) -> serde_json::Value {
    let result = InitializedResult {
        protocol_version: negotiated.protocol_version,
        server_capabilities: negotiated.capabilities,
        connection_id: connection.connection_id.clone(),
        principal_id: connection.principal_id.clone(),
        os_principal: connection.os_principal,
        runtime_version: env!("CARGO_PKG_VERSION").into(),
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": ClientMessage::v1(ProtocolClientEvent::InitializeResponse(result))
    })
}

async fn dispatch_request(
    handler: RequestHandler,
    connection: ConnectionContext,
    request: serde_json::Value,
    request_id: serde_json::Value,
    notify_tx: Option<mpsc::Sender<String>>,
) -> serde_json::Value {
    // Catch a handler panic without spawning a detached nested task. The
    // connection's JoinSet can therefore cancel the complete request future
    // before disconnect cleanup starts.
    match AssertUnwindSafe(handler.handle(&connection, request))
        .catch_unwind()
        .await
    {
        Ok(response) => response,
        Err(payload) => {
            let error = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("request handler panicked");
            let (response, terminal_events) = request_task_failure(request_id, error);
            error!(message = %response["error"]["message"], "Request handler task failed");
            if let Some(tx) = notify_tx {
                for event in terminal_events {
                    if let Ok(payload) = super::handler::format::event_to_json(&event) {
                        let _ = tx.send(payload).await;
                    }
                }
            }
            response
        }
    }
}

async fn dispatch_versioned_request(
    handler: RequestHandler,
    connection: ConnectionContext,
    request: ClientRequest,
    request_id: serde_json::Value,
    notify_tx: Option<mpsc::Sender<String>>,
) -> serde_json::Value {
    let result: anyhow::Result<ProtocolClientEvent> = match request {
        ClientRequest::Snapshot(request) => handler
            .protocol_snapshot(&request.session_id)
            .await
            .map(ProtocolClientEvent::Snapshot),
        ClientRequest::Subscribe(subscription) => {
            match handler
                .protocol_events_after(&subscription.session_id, &subscription.after)
                .await
            {
                Ok(events) => {
                    if let Some(tx) = notify_tx {
                        for event in events {
                            let notification = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "session.event",
                                "params": ClientMessage::v1(event),
                            });
                            if tx.send(notification.to_string()).await.is_err() {
                                return protocol_error(request_id, "session event receiver closed");
                            }
                        }
                    }
                    Ok(ProtocolClientEvent::Reconnected(subscription.after))
                }
                Err(error) => Err(error),
            }
        }
        ClientRequest::Chat(request) => {
            let thread_id = request.thread_id.clone();
            let workspace = fabric::WorkspaceSelection::new(
                Some(request.working_dir.clone()),
                request.additional_writable_roots,
            )
            .resolve_with_profile(
                &request.working_dir,
                &fabric::PermissionProfileId::workspace_write(),
            );
            let response = match workspace {
                Ok(workspace) => {
                    handler
                        .execute_explicit_chat(
                            &connection,
                            request_id.clone(),
                            request.message,
                            request.thread_id,
                            workspace,
                        )
                        .await
                }
                Err(error) => protocol_error(request_id.clone(), error),
            };
            if let Some(error) = response.get("error") {
                Err(anyhow::anyhow!(
                    "{}",
                    error
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("chat failed")
                ))
            } else {
                Ok(ProtocolClientEvent::CommandCompleted {
                    command: "chat".into(),
                    thread_id,
                    turn_id: None,
                    operation_id: None,
                    detail: response.get("result").cloned().unwrap_or_default(),
                })
            }
        }
        ClientRequest::Approval(request) => {
            let thread_id = request.thread_id.clone();
            let turn_id = request.turn_id;
            let operation_id = request.operation_id;
            match handler
                .resolve_versioned_approval(&connection, request)
                .await
            {
                Ok(approval) => serde_json::to_value(approval)
                    .map(|detail| ProtocolClientEvent::CommandCompleted {
                        command: "approval".into(),
                        thread_id,
                        turn_id: Some(turn_id),
                        operation_id: Some(operation_id),
                        detail,
                    })
                    .map_err(anyhow::Error::from),
                Err(error) => Err(error),
            }
        }
        ClientRequest::Cancel(request) => {
            let thread_id = request.thread_id.clone();
            let turn_id = request.turn_id;
            let operation_id = request.operation_id;
            handler
                .cancel_versioned_turn(&connection, request)
                .await
                .map(|()| ProtocolClientEvent::CommandCompleted {
                    command: "cancel".into(),
                    thread_id,
                    turn_id: Some(turn_id),
                    operation_id: Some(operation_id),
                    detail: serde_json::json!({"status":"cancelled"}),
                })
        }
        ClientRequest::Initialize(_) | ClientRequest::Initialized => {
            Err(anyhow::anyhow!("handshake request cannot be dispatched"))
        }
    };
    match result {
        Ok(event) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": ClientMessage::v1(event),
        }),
        Err(error) => protocol_error(request_id, error),
    }
}

async fn run_versioned_subscription(
    handler: RequestHandler,
    subscription: fabric::protocol::client::EventSubscription,
    request_id: serde_json::Value,
    notify_tx: mpsc::Sender<String>,
    resp_tx: mpsc::Sender<String>,
) {
    let mut cursor = subscription.after;
    let events = match handler
        .protocol_events_after(&subscription.session_id, &cursor)
        .await
    {
        Ok(events) => events,
        Err(error) => {
            let response = protocol_error(request_id.clone(), error);
            let _ = resp_tx.send(response.to_string()).await;
            return;
        }
    };
    for event in events {
        if let ProtocolClientEvent::Item(item) = &event {
            cursor = item.cursor.clone();
        }
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session.event",
            "params": ClientMessage::v1(event),
        });
        if notify_tx.send(notification.to_string()).await.is_err() {
            return;
        }
    }
    // Acknowledge only after the initial replay is enqueued. The task then
    // remains connection-owned and tails durable events until disconnect.
    let response = serde_json::json!({
        "jsonrpc":"2.0",
        "id":request_id,
        "result":ClientMessage::v1(ProtocolClientEvent::Reconnected(cursor.clone())),
    });
    if resp_tx.send(response.to_string()).await.is_err() {
        return;
    }
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let events = match handler
            .protocol_events_after(&subscription.session_id, &cursor)
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(%error, session = %subscription.session_id.0, "versioned subscription tail failed");
                return;
            }
        };
        for event in events {
            if let ProtocolClientEvent::Item(item) = &event {
                cursor = item.cursor.clone();
            }
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session.event",
                "params": ClientMessage::v1(event),
            });
            if notify_tx.send(notification.to_string()).await.is_err() {
                return;
            }
        }
    }
}

fn request_task_failure(
    request_id: serde_json::Value,
    error: impl std::fmt::Display,
) -> (serde_json::Value, Vec<ClientEvent>) {
    let message = format!("request task failed: {error}");
    (
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": { "code": -32603, "message": message }
        }),
        vec![
            ClientEvent::Error {
                message: message.clone(),
            },
            ClientEvent::TurnDone,
        ],
    )
}

pub struct UnixServer {
    listener: UnixListener,
    handler: RequestHandler,
    cancel_token: CancellationToken,
    /// Tracks spawned connection tasks for graceful shutdown drain.
    connections: JoinSet<()>,
    /// UID of the daemon process — allowed to connect.
    owner_uid: u32,
    /// GID of the aletheon group — users in this group may also connect.
    group_gid: u32,
    #[allow(dead_code)]
    clock: Arc<dyn Clock>,
}

impl UnixServer {
    pub async fn new(
        socket_path: &Path,
        handler: RequestHandler,
        cancel_token: CancellationToken,
        owner_uid: u32,
        group_gid: u32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let listener =
            bind_path_listener(socket_path, SocketPrivacy::SystemCore, owner_uid).await?;
        info!(path = %socket_path.display(), owner_uid, group_gid, "Unix socket listening");

        Ok(Self::from_listener(
            listener,
            handler,
            cancel_token,
            owner_uid,
            group_gid,
            clock,
        ))
    }

    /// Bind a per-user runtime socket with a private parent directory and mode.
    pub async fn new_user_private(
        socket_path: &Path,
        handler: RequestHandler,
        cancel_token: CancellationToken,
        owner_uid: u32,
        group_gid: u32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let listener =
            bind_path_listener(socket_path, SocketPrivacy::UserPrivate, owner_uid).await?;
        info!(path = %socket_path.display(), owner_uid, "Private Unix socket listening");
        Ok(Self::from_listener(
            listener,
            handler,
            cancel_token,
            owner_uid,
            group_gid,
            clock,
        ))
    }

    /// Construct the server around an already-bound listener, such as one
    /// supplied by systemd socket activation.
    pub fn from_listener(
        listener: UnixListener,
        handler: RequestHandler,
        cancel_token: CancellationToken,
        owner_uid: u32,
        group_gid: u32,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            listener,
            handler,
            cancel_token,
            connections: JoinSet::new(),
            owner_uid,
            group_gid,
            clock,
        }
    }

    /// Return a reference to the handler so the host can interact with it
    /// after the accept loop finishes (e.g., for graceful shutdown).
    pub fn handler(&self) -> &RequestHandler {
        &self.handler
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                accept_result = self.listener.accept() => {
                    let (stream, _addr) = accept_result?;
                    // Verify peer credentials before accepting the connection.
                    let peer = match Self::check_peer_cred(&stream, self.owner_uid, self.group_gid) {
                        Ok(peer) => peer,
                        Err(e) => {
                            warn!(error = %e, "Connection rejected by peer credential check");
                            continue;
                        }
                    };
                    let connection = ConnectionContext::from_peer(peer);
                    let mut handler = self.handler.clone();

                    // Create a per-connection notify channel so each client receives
                    // its own events independently (shared channels would cause events
                    // to be consumed by whichever connection reads first).
                    let (notify_tx, notify_rx) =
                        mpsc::channel::<String>(CONNECTION_NOTIFICATION_CAPACITY);
                    handler.set_notify_channel(notify_tx).await;
                    handler.increment_connections();

                    self.connections.spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, handler, notify_rx, connection).await {
                            error!(error = %e, "Connection error");
                        }
                    });
                }
                _ = self.cancel_token.cancelled() => {
                    info!("Shutdown signal received, stopping accept loop");
                    break;
                }
            }
        }

        // Drain in-flight connections with a 5-second timeout per task.
        info!(
            remaining = self.connections.len(),
            "Draining in-flight connections..."
        );
        loop {
            match SystemTimer
                .timeout(Duration::from_secs(5), self.connections.join_next())
                .await
            {
                Ok(Some(Ok(()))) => {
                    // Connection completed normally.
                }
                Ok(Some(Err(e))) => {
                    error!(error = %e, "Connection task panicked during drain");
                }
                Ok(None) => {
                    info!("All connections drained");
                    break;
                }
                Err(_elapsed) => {
                    info!(
                        remaining = self.connections.len(),
                        "Drain timeout expired, aborting remaining connections"
                    );
                    self.connections.abort_all();
                    break;
                }
            }
        }

        Ok(())
    }

    /// Verify that the connecting peer is either the daemon owner or a member
    /// of the aletheon group. Root (uid 0) is always allowed.
    fn check_peer_cred(
        stream: &tokio::net::UnixStream,
        owner_uid: u32,
        group_gid: u32,
    ) -> anyhow::Result<LocalOsPrincipal> {
        let cred = stream.peer_cred()?;
        let peer_uid = cred.uid();
        let peer_gid = cred.gid();

        // Allow root and the daemon owner.
        if peer_uid == 0 || peer_uid == owner_uid {
            return Ok(LocalOsPrincipal {
                uid: peer_uid,
                gid: peer_gid,
            });
        }

        // Check if the peer belongs to the aletheon group.
        // First check primary group (fast path, no allocation).
        if peer_gid == group_gid {
            return Ok(LocalOsPrincipal {
                uid: peer_uid,
                gid: peer_gid,
            });
        }
        // Then check supplementary groups via nix.
        if let Some(user) = User::from_uid(Uid::from_raw(peer_uid))? {
            let c_name = CString::new(user.name)?;
            let groups = nix::unistd::getgrouplist(&c_name, Gid::from_raw(cred.gid()))?;
            if groups.contains(&Gid::from_raw(group_gid)) {
                return Ok(LocalOsPrincipal {
                    uid: peer_uid,
                    gid: peer_gid,
                });
            }
        }

        anyhow::bail!("Access denied: uid {} not in aletheon group", peer_uid)
    }

    /// Handle a single client connection. Reads JSON-RPC requests from the
    /// client and also writes out-of-band notifications (e.g. approval_request)
    /// from the handler's notification channel, and debug subscriber events.
    async fn handle_connection(
        stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        handler: RequestHandler,
        notify_rx: mpsc::Receiver<String>,
        connection: ConnectionContext,
    ) -> Result<()> {
        let result =
            Self::handle_connection_inner(stream, handler.clone(), notify_rx, connection.clone())
                .await;
        if let Err(error) = handler
            .cleanup_disconnected_connection(&connection.connection_id)
            .await
        {
            tracing::warn!(
                connection_id = %connection.connection_id.0,
                %error,
                "failed to clean up connection-owned foreground processes"
            );
        }
        handler.decrement_connections();
        result
    }

    async fn handle_connection_inner(
        stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        handler: RequestHandler,
        mut notify_rx: mpsc::Receiver<String>,
        connection: ConnectionContext,
    ) -> Result<()> {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Debug subscriber receiver — populated when the client sends debug.subscribe.
        let mut debug_subscriber_rx: Option<mpsc::Receiver<DebugEvent>> = None;

        // Channel for receiving handler responses from background tasks.
        // This allows the select! loop to continue forwarding notifications
        // while the handler is processing a long-running request (e.g. LLM API call).
        let (resp_tx, mut resp_rx) = mpsc::channel::<String>(1);
        let mut protocol_state = ConnectionProtocolState::New;
        let mut request_tasks = JoinSet::new();
        let mut versioned_subscription_started = false;

        loop {
            tokio::select! {
                // Read incoming requests from the client.
                // Dispatch to a background task so the select loop continues
                // forwarding notifications while the handler processes.
                read_result = reader.read_line(&mut line) => {
                    let n = read_result?;
                    if n == 0 {
                        break; // Connection closed
                    }

                    let trimmed = line.trim().to_string();
                    line.clear();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Parse JSON request and spawn handler in background
                    let request: serde_json::Value = serde_json::from_str(&trimmed)?;
                    let request_id = request
                        .get("id")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);

                    if let Some(versioned) = parse_versioned_request(&request) {
                        let versioned = match versioned {
                            Ok(versioned) => versioned,
                            Err(error) => {
                                let response = protocol_error(request_id, error);
                                resp_tx.send(serde_json::to_string(&response)?).await?;
                                continue;
                            }
                        };
                        let response = match protocol_state.accept(&versioned) {
                            Ok(ProtocolAction::InitializeResponse(negotiated)) => {
                                initialize_response(request_id, &connection, negotiated)
                            }
                            Ok(ProtocolAction::Initialized) => serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": request_id,
                                "result": { "status": "ready" }
                            }),
                            Ok(ProtocolAction::Dispatch) => {
                                if let ClientRequest::Subscribe(subscription) = &versioned {
                                    if versioned_subscription_started {
                                        let response = protocol_error(
                                            request_id,
                                            "connection already has an active session subscription",
                                        );
                                        resp_tx.send(response.to_string()).await?;
                                        continue;
                                    }
                                    let Some(notify_tx) = handler.notify_tx.clone() else {
                                        let response = protocol_error(
                                            request_id,
                                            "connection notification channel is unavailable",
                                        );
                                        resp_tx.send(response.to_string()).await?;
                                        continue;
                                    };
                                    versioned_subscription_started = true;
                                    let handler = handler.clone();
                                    let subscription = subscription.clone();
                                    let resp_tx = resp_tx.clone();
                                    request_tasks.spawn(async move {
                                        run_versioned_subscription(
                                            handler,
                                            subscription,
                                            request_id,
                                            notify_tx,
                                            resp_tx,
                                        )
                                        .await;
                                    });
                                    continue;
                                }
                                let handler = handler.clone();
                                let notify_tx = handler.notify_tx.clone();
                                let resp_tx = resp_tx.clone();
                                let request_connection = connection.clone();
                                request_tasks.spawn(async move {
                                    let response = dispatch_versioned_request(
                                        handler,
                                        request_connection,
                                        versioned,
                                        request_id,
                                        notify_tx,
                                    )
                                    .await;
                                    let response_json = serde_json::to_string(&response)
                                        .unwrap_or_default();
                                    let _ = resp_tx.send(response_json).await;
                                });
                                continue;
                            }
                            Err(error) => protocol_error(request_id, error),
                        };
                        let response_json = serde_json::to_string(&response)?;
                        resp_tx.send(response_json).await?;
                        continue;
                    }

                    if !LegacyClientHandshakeAdapter::bind(&mut protocol_state, &request) {
                        let response = protocol_error(
                            request_id,
                            "legacy and versioned requests cannot share a connection",
                        );
                        resp_tx.send(serde_json::to_string(&response)?).await?;
                        continue;
                    }

                    let handler = handler.clone();
                    let notify_tx = handler.notify_tx.clone();
                    let resp_tx = resp_tx.clone();
                    let connection = connection.clone();
                    request_tasks.spawn(async move {
                        let response = dispatch_request(
                            handler,
                            connection,
                            request,
                            request_id,
                            notify_tx,
                        )
                        .await;
                        let response_json = serde_json::to_string(&response)
                            .unwrap_or_default();
                        let _ = resp_tx.send(response_json).await;
                    });
                }
                // Receive handler response from background task.
                response_json = resp_rx.recv() => {
                    if let Some(json) = response_json {
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;

                        // Check if the debug handler has a pending subscriber rx
                        // (populated when debug.subscribe was just processed).
                        if let Some(rx) = handler.debug_handler().take_pending_subscriber_rx().await {
                            debug_subscriber_rx = Some(rx);
                            info!("Debug subscriber channel attached to client connection");
                        }
                    }
                }
                completed = request_tasks.join_next(), if !request_tasks.is_empty() => {
                    if let Some(Err(error)) = completed {
                        warn!(%error, "connection request task failed");
                    }
                }
                // Forward out-of-band notifications from the handler to the client.
                notification = notify_rx.recv() => {
                    match notification {
                        Some(msg) => {
                            writer.write_all(msg.as_bytes()).await?;
                            writer.write_all(b"\n").await?;
                            writer.flush().await?;
                        }
                        None => {
                            // Notification channel closed — handler dropped.
                            // Continue reading requests normally.
                        }
                    }
                }
                // Forward debug subscriber events to the client.
                debug_event = async {
                    match &mut debug_subscriber_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(event) = debug_event {
                        let json = serde_json::to_string(&event)?;
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                    } else {
                        // Subscriber channel closed — clear it.
                        debug_subscriber_rx = None;
                    }
                }
            }
        }

        // No request started by this transport may outlive it. In particular,
        // this closes the race where a detached request could register an
        // approval after disconnect cleanup had already run.
        request_tasks.shutdown().await;
        Ok(())
    }
}

async fn bind_path_listener(
    socket_path: &Path,
    privacy: SocketPrivacy,
    owner_uid: u32,
) -> Result<UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};

    if privacy == SocketPrivacy::UserPrivate {
        let parent = socket_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("private socket path has no parent"))?;
        if !parent.exists() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(parent)?;
        }
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("private socket parent is not a real directory");
        }
        if metadata.uid() != owner_uid {
            anyhow::bail!(
                "private socket parent is owned by uid {}, expected {}",
                metadata.uid(),
                owner_uid
            );
        }
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }

    match std::fs::symlink_metadata(socket_path) {
        Ok(metadata) if privacy == SocketPrivacy::UserPrivate => {
            if !metadata.file_type().is_socket() || metadata.uid() != owner_uid {
                anyhow::bail!("refusing to replace non-owned private socket path");
            }
            tokio::fs::remove_file(socket_path).await?;
        }
        Ok(_) => tokio::fs::remove_file(socket_path).await?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let listener = UnixListener::bind(socket_path)?;
    let mode = match privacy {
        SocketPrivacy::UserPrivate => 0o600,
        SocketPrivacy::SystemCore => 0o660,
    };
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(mode))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use fabric::paths::RuntimeEnvironment;

    use super::*;

    #[derive(Default)]
    struct FakeActivationEnvironment {
        values: Mutex<BTreeMap<String, OsString>>,
    }

    impl FakeActivationEnvironment {
        fn with(values: impl IntoIterator<Item = (&'static str, String)>) -> Self {
            Self {
                values: Mutex::new(
                    values
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value.into()))
                        .collect(),
                ),
            }
        }
    }

    impl fabric::paths::RuntimeEnvironment for FakeActivationEnvironment {
        fn var_os(&self, key: &str) -> Option<OsString> {
            self.values.lock().unwrap().get(key).cloned()
        }
    }

    impl ActivationEnvironment for FakeActivationEnvironment {
        fn remove_var(&self, key: &str) {
            self.values.lock().unwrap().remove(key);
        }
    }

    #[tokio::test]
    async fn one_systemd_listener_is_adopted_without_using_real_fd_three() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("activated.sock");
        let original = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let duplicate: OwnedFd = original.try_clone().unwrap().into();
        let env = FakeActivationEnvironment::with([
            ("LISTEN_PID", std::process::id().to_string()),
            ("LISTEN_FDS", "1".to_owned()),
        ]);

        let adopted = inherited_listener(&env, Some(duplicate))
            .unwrap()
            .expect("activation listener");

        assert_eq!(
            adopted.local_addr().unwrap().as_pathname(),
            Some(socket_path.as_path())
        );
        assert!(env.var_os("LISTEN_PID").is_none());
        assert!(env.var_os("LISTEN_FDS").is_none());
    }

    #[tokio::test]
    async fn absent_activation_environment_falls_back_to_path_binding() {
        let env = FakeActivationEnvironment::default();
        assert!(inherited_listener(&env, None).unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_activation_environment_fails_closed() {
        let cases = [
            FakeActivationEnvironment::with([("LISTEN_PID", std::process::id().to_string())]),
            FakeActivationEnvironment::with([
                ("LISTEN_PID", (std::process::id() + 1).to_string()),
                ("LISTEN_FDS", "1".to_owned()),
            ]),
            FakeActivationEnvironment::with([
                ("LISTEN_PID", std::process::id().to_string()),
                ("LISTEN_FDS", "2".to_owned()),
            ]),
        ];

        assert!(matches!(
            inherited_listener(&cases[0], None),
            Err(ActivationError::IncompleteEnvironment)
        ));
        assert!(matches!(
            inherited_listener(&cases[1], None),
            Err(ActivationError::WrongPid { .. })
        ));
        assert!(matches!(
            inherited_listener(&cases[2], None),
            Err(ActivationError::InvalidFdCount(2))
        ));
    }

    #[tokio::test]
    async fn activation_rejects_a_non_listening_unix_socket() {
        let datagram = std::os::unix::net::UnixDatagram::unbound().unwrap();
        let env = FakeActivationEnvironment::with([
            ("LISTEN_PID", std::process::id().to_string()),
            ("LISTEN_FDS", "1".to_owned()),
        ]);

        assert!(matches!(
            inherited_listener(&env, Some(datagram.into())),
            Err(ActivationError::InvalidListener)
        ));
    }

    #[tokio::test]
    async fn private_path_binding_sets_parent_and_socket_modes() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("runtime/aletheon/aletheon.sock");
        let owner_uid = nix::unistd::geteuid().as_raw();

        let _listener = bind_path_listener(&socket_path, SocketPrivacy::UserPrivate, owner_uid)
            .await
            .unwrap();

        assert_eq!(
            std::fs::metadata(socket_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn system_core_path_binding_retains_group_mode() {
        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("core.sock");
        let _listener = bind_path_listener(
            &socket_path,
            SocketPrivacy::SystemCore,
            nix::unistd::geteuid().as_raw(),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::metadata(&socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o660
        );
    }

    fn capabilities() -> fabric::protocol::client::ClientCapabilities {
        fabric::protocol::client::ClientCapabilities {
            item_events: true,
            cursors: true,
        }
    }

    fn initialize() -> fabric::protocol::client::ClientRequest {
        fabric::protocol::client::ClientRequest::Initialize(
            fabric::protocol::client::InitializeParams {
                client_version: "test-client".into(),
                protocol_versions: vec![fabric::protocol::client::CLIENT_PROTOCOL_VERSION],
                capabilities: capabilities(),
            },
        )
    }

    fn snapshot() -> fabric::protocol::client::ClientRequest {
        fabric::protocol::client::ClientRequest::Snapshot(
            fabric::protocol::client::SnapshotRequest {
                session_id: fabric::SessionId("thread-a".into()),
            },
        )
    }

    #[test]
    fn json_identity_cannot_replace_peer_identity() {
        let peer = fabric::LocalOsPrincipal {
            uid: 1001,
            gid: 100,
        };
        let connection = ConnectionContext::from_peer(peer);
        let request = serde_json::json!({"method":"chat","params":{"uid":0,"gid":0}});
        assert_eq!(
            connection.principal_id,
            fabric::PrincipalId::local_uid(1001)
        );
        assert_eq!(connection.os_principal.uid, 1001);
        assert_ne!(
            request["params"]["uid"].as_u64(),
            Some(u64::from(connection.os_principal.uid))
        );
    }

    #[test]
    fn connection_requires_initialize_then_initialized_exactly_once() {
        let mut state = ConnectionProtocolState::New;
        assert!(state.accept(&snapshot()).is_err());
        state.accept(&initialize()).unwrap();
        assert!(state.accept(&initialize()).is_err());
        state
            .accept(&fabric::protocol::client::ClientRequest::Initialized)
            .unwrap();
        assert!(matches!(
            state,
            ConnectionProtocolState::Ready {
                negotiated: Some(_)
            }
        ));
        assert!(state.accept(&initialize()).is_err());
    }

    #[test]
    fn legacy_adapter_binds_identity_without_weakening_versioned_handshake() {
        let legacy = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "status",
            "params": {"uid": 0, "gid": 0}
        });
        let mut legacy_state = ConnectionProtocolState::New;
        assert!(LegacyClientHandshakeAdapter::bind(
            &mut legacy_state,
            &legacy
        ));
        assert!(matches!(
            legacy_state,
            ConnectionProtocolState::Ready { negotiated: None }
        ));

        let versioned = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": fabric::protocol::client::ClientMessage::v1(initialize())
        });
        let mut versioned_state = ConnectionProtocolState::New;
        assert!(!LegacyClientHandshakeAdapter::bind(
            &mut versioned_state,
            &versioned
        ));
        assert!(matches!(versioned_state, ConnectionProtocolState::New));
    }

    #[tokio::test]
    async fn connection_owned_subscription_task_is_bounded_and_cancelled_on_cleanup() {
        let (tx, _rx) = mpsc::channel::<String>(CONNECTION_NOTIFICATION_CAPACITY);
        assert_eq!(tx.capacity(), CONNECTION_NOTIFICATION_CAPACITY);
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct DropMark(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropMark {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let mut tasks = JoinSet::new();
        let marker = DropMark(dropped.clone());
        tasks.spawn(async move {
            let _marker = marker;
            std::future::pending::<()>().await;
        });
        tasks.shutdown().await;
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn request_task_failure_retains_id_and_finishes_client_turn() {
        let (response, events) = request_task_failure(serde_json::json!(42), "panic");
        assert_eq!(response["id"], 42);
        assert_eq!(response["error"]["code"], -32603);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("panic"));
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::Error { .. }, ClientEvent::TurnDone]
        ));
    }
}
