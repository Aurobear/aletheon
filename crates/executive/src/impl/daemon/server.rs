use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use aletheon_kernel::chronos::SystemTimer;
use anyhow::Result;
use fabric::debug::DebugEvent;
use fabric::events::ui_event::ClientEvent;
use fabric::protocol::client::{
    negotiate_protocol_version, ClientCapabilities, ClientEvent as ProtocolClientEvent,
    ClientMessage, ClientRequest, InitializedResult,
};
use fabric::{Clock, ConnectionId, LocalOsPrincipal, PrincipalId, Timer};
use nix::unistd::{Gid, Uid, User};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::handler::RequestHandler;

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
    let task = tokio::spawn(async move { handler.handle(&connection, request).await });
    match task.await {
        Ok(response) => response,
        Err(error) => {
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
        // Remove stale socket
        if socket_path.exists() {
            tokio::fs::remove_file(socket_path).await?;
        }

        let listener = UnixListener::bind(socket_path)?;
        // Restrict socket to owner and group only (rw-rw----).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
        }
        info!(path = %socket_path.display(), owner_uid, group_gid, "Unix socket listening");

        Ok(Self {
            listener,
            handler,
            cancel_token,
            connections: JoinSet::new(),
            owner_uid,
            group_gid,
            clock,
        })
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
                    let (notify_tx, notify_rx) = mpsc::channel::<String>(64);
                    handler.set_notify_channel(notify_tx);
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
                        let response = match versioned.and_then(|request| protocol_state.accept(&request)) {
                            Ok(ProtocolAction::InitializeResponse(negotiated)) => {
                                initialize_response(request_id, &connection, negotiated)
                            }
                            Ok(ProtocolAction::Initialized) => serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": request_id,
                                "result": { "status": "ready" }
                            }),
                            Ok(ProtocolAction::Dispatch) => {
                                let handler = handler.clone();
                                let notify_tx = handler.notify_tx.clone();
                                let resp_tx = resp_tx.clone();
                                let connection = connection.clone();
                                tokio::spawn(async move {
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
                    tokio::spawn(async move {
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

        handler.decrement_connections();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
