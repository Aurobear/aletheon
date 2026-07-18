//! Authenticated ACP request dispatch and event-stream gateway.

use std::{io, path::Path};

use async_trait::async_trait;
use fabric::{protocol::client::ClientEvent, PrincipalContext, ThreadId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncWrite};

use super::{
    map_client_event_to_acp, transport::AcpTransport, AcpAdapter, AcpError, AcpRequest, AcpResponse,
};

/// Authentication is a required constructor input, never an optional fallback.
/// The Executive daemon must construct this after peer-credential validation.
#[derive(Debug, Clone)]
pub struct AuthenticatedAcpConnection {
    principal: PrincipalContext,
}

impl AuthenticatedAcpConnection {
    pub fn new(principal: PrincipalContext) -> Self {
        Self { principal }
    }

    pub fn principal(&self) -> &PrincipalContext {
        &self.principal
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedAcpSession {
    pub session_id: String,
    pub thread_id: ThreadId,
}

/// Executive boundary used by ACP. Implementations must delegate to the
/// authoritative session, prompt-queue/turn, and cancellation use cases.
#[async_trait]
pub trait AcpBackend: Send + Sync {
    async fn create_session(
        &self,
        principal: &PrincipalContext,
        cwd: &Path,
    ) -> Result<CreatedAcpSession, AcpError>;

    async fn submit_prompt(
        &self,
        principal: &PrincipalContext,
        session_id: &str,
        thread_id: &ThreadId,
        text: &str,
    ) -> Result<(), AcpError>;

    async fn cancel_turn(
        &self,
        principal: &PrincipalContext,
        session_id: &str,
        thread_id: &ThreadId,
    ) -> Result<(), AcpError>;
}

/// One authoritative event tagged with the ACP session lookup key. Event
/// sources retain the real history/cursor; the adapter owns no replay state.
#[derive(Debug, Clone)]
pub struct AcpSessionEvent {
    pub session_id: String,
    pub event: ClientEvent,
}

#[async_trait]
pub trait AcpEventSource: Send {
    async fn next_event(&mut self) -> Result<Option<AcpSessionEvent>, AcpError>;

    /// Rebuild an authoritative snapshot and replay only events after `cursor`.
    async fn recover(
        &mut self,
        _session_id: &str,
        _cursor: &fabric::protocol::client::EventCursor,
    ) -> Result<Vec<AcpSessionEvent>, AcpError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpServerFrame {
    Response {
        request_id: u64,
        response: AcpResponse,
    },
    SessionUpdate {
        session_id: String,
        update: Value,
    },
}

#[derive(Debug, Deserialize)]
struct AcpClientFrame {
    request_id: u64,
    request: AcpRequest,
}

impl AcpAdapter {
    pub async fn dispatch<B: AcpBackend>(
        &mut self,
        connection: &AuthenticatedAcpConnection,
        backend: &B,
        request: AcpRequest,
    ) -> AcpResponse {
        match self.try_dispatch(connection, backend, request).await {
            Ok(response) => response,
            Err(error) => AcpResponse::Error {
                message: error.to_string(),
            },
        }
    }

    async fn try_dispatch<B: AcpBackend>(
        &mut self,
        connection: &AuthenticatedAcpConnection,
        backend: &B,
        request: AcpRequest,
    ) -> Result<AcpResponse, AcpError> {
        let principal = connection.principal();
        match request {
            AcpRequest::Initialize {
                protocol_versions, ..
            } => Ok(self.initialize(&protocol_versions)),
            AcpRequest::NewSession { cwd } => {
                let cwd = canonical_authorized_workspace(principal, &cwd)?;
                let created = backend.create_session(principal, &cwd).await?;
                let response = self.bind_created_session(
                    created.session_id,
                    principal.connection_id.clone(),
                    created.thread_id,
                );
                self.metrics.sessions_active = self.metrics.sessions_active.saturating_add(1);
                Ok(response)
            }
            AcpRequest::Prompt { session_id, text } => {
                if text.trim().is_empty() {
                    return Err(AcpError::InvalidPrompt);
                }
                let binding = self
                    .resolve_session(&session_id, &principal.connection_id)?
                    .clone();
                backend
                    .submit_prompt(principal, &session_id, &binding.thread_id, &text)
                    .await?;
                self.metrics.prompt_total = self.metrics.prompt_total.saturating_add(1);
                Ok(AcpResponse::Accepted)
            }
            AcpRequest::Cancel { session_id } => {
                let binding = self
                    .resolve_session(&session_id, &principal.connection_id)?
                    .clone();
                backend
                    .cancel_turn(principal, &session_id, &binding.thread_id)
                    .await?;
                Ok(AcpResponse::Cancelled)
            }
        }
    }

    fn map_authorized_event(
        &mut self,
        connection: &AuthenticatedAcpConnection,
        event: AcpSessionEvent,
    ) -> Result<Option<AcpServerFrame>, AcpError> {
        self.resolve_session(&event.session_id, &connection.principal().connection_id)?;
        if matches!(&event.event, ClientEvent::Reconnected(_)) {
            self.metrics.reconnect_total = self.metrics.reconnect_total.saturating_add(1);
        }
        let mapped = map_client_event_to_acp(&event.event);
        if mapped.is_none() {
            self.metrics.map_unmapped_event_total =
                self.metrics.map_unmapped_event_total.saturating_add(1);
        }
        Ok(mapped.map(|update| AcpServerFrame::SessionUpdate {
            session_id: event.session_id,
            update,
        }))
    }
}

fn canonical_authorized_workspace(
    principal: &PrincipalContext,
    requested: &Path,
) -> Result<std::path::PathBuf, AcpError> {
    let canonical =
        std::fs::canonicalize(requested).map_err(|_| AcpError::WorkspaceNotAuthorized)?;
    if !canonical.is_dir()
        || !principal
            .workspace
            .writable_roots()
            .iter()
            .any(|root| canonical.starts_with(root))
    {
        return Err(AcpError::WorkspaceNotAuthorized);
    }
    Ok(canonical)
}

/// Run a bounded, multiplexed gateway: requests remain readable while prompt
/// events stream, so a cancel frame is not blocked behind the active turn.
pub async fn run_transport_loop<R, W, B, E>(
    adapter: &mut AcpAdapter,
    connection: &AuthenticatedAcpConnection,
    backend: &B,
    events: &mut E,
    transport: &mut AcpTransport<R, W>,
) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    B: AcpBackend,
    E: AcpEventSource,
{
    let mut events_open = true;
    loop {
        tokio::select! {
            frame = transport.read_frame::<AcpClientFrame>() => {
                let frame = match frame {
                    Ok(Some(frame)) => frame,
                    Ok(None) => {
                        adapter.metrics.sessions_active = 0;
                        return Ok(());
                    }
                    Err(error) => {
                        tracing::warn!(event="acp.request.rejected", reason=%error, "unknown or invalid ACP request");
                        return Err(error);
                    }
                };
                let response = adapter.dispatch(connection, backend, frame.request).await;
                transport.write_frame(&AcpServerFrame::Response {
                    request_id: frame.request_id,
                    response,
                }).await?;
            }
            event = events.next_event(), if events_open => {
                let event = event.map_err(acp_io_error)?;
                let Some(event) = event else {
                    events_open = false;
                    continue;
                };
                if let ClientEvent::Reconnected(cursor) = &event.event {
                    let recovered = events.recover(&event.session_id, cursor).await.map_err(acp_io_error)?;
                    for recovered_event in recovered {
                        match adapter.map_authorized_event(connection, recovered_event) {
                            Ok(Some(frame)) => transport.write_frame(&frame).await?,
                            Ok(None) => {}
                            Err(error) => return Err(acp_io_error(error)),
                        }
                    }
                    // The authoritative snapshot and replay supersede the stale
                    // reconnect cursor; emitting it afterwards would move the
                    // client-visible high-water mark backwards.
                    continue;
                }
                match adapter.map_authorized_event(connection, event) {
                    Ok(Some(frame)) => transport.write_frame(&frame).await?,
                    Ok(None) => {}
                    // A source must never be able to leak a cross-connection event.
                    Err(error) => return Err(acp_io_error(error)),
                }
            }
        }
    }
}

fn acp_io_error(error: AcpError) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;
    use fabric::{
        protocol::client::{EventCursor, ItemEvent, ItemPhase},
        ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId, PrincipalId,
        WorkspacePolicy,
    };
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
    }

    struct FakeEvents(VecDeque<AcpSessionEvent>);

    #[async_trait]
    impl AcpEventSource for FakeEvents {
        async fn next_event(&mut self) -> Result<Option<AcpSessionEvent>, AcpError> {
            Ok(self.0.pop_front())
        }
    }

    #[async_trait]
    impl AcpBackend for FakeBackend {
        async fn create_session(
            &self,
            _principal: &PrincipalContext,
            _cwd: &Path,
        ) -> Result<CreatedAcpSession, AcpError> {
            self.calls.lock().unwrap().push("create".into());
            Ok(CreatedAcpSession {
                session_id: "session-1".into(),
                thread_id: ThreadId("thread-1".into()),
            })
        }

        async fn submit_prompt(
            &self,
            _principal: &PrincipalContext,
            session_id: &str,
            thread_id: &ThreadId,
            _text: &str,
        ) -> Result<(), AcpError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("prompt:{session_id}:{}", thread_id.0));
            Ok(())
        }

        async fn cancel_turn(
            &self,
            _principal: &PrincipalContext,
            session_id: &str,
            thread_id: &ThreadId,
        ) -> Result<(), AcpError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("cancel:{session_id}:{}", thread_id.0));
            Ok(())
        }
    }

    fn connection(root: &Path) -> AuthenticatedAcpConnection {
        let workspace = WorkspacePolicy::from_resolved_roots(root.to_path_buf(), vec![]).unwrap();
        AuthenticatedAcpConnection::new(PrincipalContext::new(
            PrincipalId::local_uid(501),
            LocalOsPrincipal { uid: 501, gid: 20 },
            ConnectionId::new(),
            ThreadId("connection-thread".into()),
            workspace,
            PermissionProfileId::workspace_write(),
            ApprovalPolicy::OnRequest,
        ))
    }

    #[tokio::test]
    async fn dispatch_routes_create_prompt_and_cancel_through_authoritative_port() {
        let root = tempfile::tempdir().unwrap();
        let connection = connection(root.path());
        let backend = FakeBackend::default();
        let mut adapter = AcpAdapter::default();

        assert!(matches!(
            adapter
                .dispatch(
                    &connection,
                    &backend,
                    AcpRequest::NewSession {
                        cwd: root.path().to_path_buf()
                    }
                )
                .await,
            AcpResponse::SessionCreated { .. }
        ));
        assert_eq!(
            adapter
                .dispatch(
                    &connection,
                    &backend,
                    AcpRequest::Prompt {
                        session_id: "session-1".into(),
                        text: "hello".into()
                    }
                )
                .await,
            AcpResponse::Accepted
        );
        assert_eq!(
            adapter
                .dispatch(
                    &connection,
                    &backend,
                    AcpRequest::Cancel {
                        session_id: "session-1".into()
                    }
                )
                .await,
            AcpResponse::Cancelled
        );
        assert_eq!(
            *backend.calls.lock().unwrap(),
            [
                "create",
                "prompt:session-1:thread-1",
                "cancel:session-1:thread-1"
            ]
        );
        assert_eq!(adapter.metrics().sessions_active, 1);
        assert_eq!(adapter.metrics().prompt_total, 1);
    }

    #[tokio::test]
    async fn dispatch_rejects_unbound_session_without_calling_backend() {
        let root = tempfile::tempdir().unwrap();
        let connection = connection(root.path());
        let backend = FakeBackend::default();
        let mut adapter = AcpAdapter::default();
        assert!(matches!(
            adapter
                .dispatch(
                    &connection,
                    &backend,
                    AcpRequest::Prompt {
                        session_id: "client-invented".into(),
                        text: "hello".into()
                    }
                )
                .await,
            AcpResponse::Error { .. }
        ));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_rejects_workspace_outside_authenticated_roots() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let connection = connection(root.path());
        let backend = FakeBackend::default();
        let mut adapter = AcpAdapter::default();
        assert!(matches!(
            adapter
                .dispatch(
                    &connection,
                    &backend,
                    AcpRequest::NewSession {
                        cwd: outside.path().to_path_buf()
                    }
                )
                .await,
            AcpResponse::Error { .. }
        ));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transport_loop_multiplexes_response_and_authorized_event_update() {
        let root = tempfile::tempdir().unwrap();
        let connection = connection(root.path());
        let backend = FakeBackend::default();
        let mut adapter = AcpAdapter::default();
        adapter.bind_created_session(
            "session-1".into(),
            connection.principal().connection_id.clone(),
            ThreadId("thread-1".into()),
        );
        let mut events = FakeEvents(VecDeque::from([AcpSessionEvent {
            session_id: "session-1".into(),
            event: ClientEvent::Item(ItemEvent {
                cursor: EventCursor {
                    sequence: 1,
                    event_id: None,
                },
                item_id: "item-1".into(),
                phase: ItemPhase::Streaming,
                delta: Some("chunk".into()),
                item: None,
                error: None,
            }),
        }]));
        let (mut client_write, server_read) = duplex(2048);
        let (server_write, client_read) = duplex(2048);
        let mut transport = AcpTransport::new(BufReader::new(server_read), server_write);

        let gateway = tokio::spawn(async move {
            run_transport_loop(
                &mut adapter,
                &connection,
                &backend,
                &mut events,
                &mut transport,
            )
            .await
        });
        client_write
            .write_all(
                b"{\"request_id\":7,\"request\":{\"method\":\"initialize\",\"params\":{\"client_capabilities\":{},\"protocol_versions\":[1]}}}\n",
            )
            .await
            .unwrap();

        let mut reader = BufReader::new(client_read);
        let mut lines = Vec::new();
        for _ in 0..2 {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            lines.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        assert!(lines
            .iter()
            .any(|frame| frame["type"] == "response" && frame["request_id"] == 7));
        assert!(lines.iter().any(|frame| frame["type"] == "session_update"
            && frame["update"]["content"]["text"] == "chunk"));

        client_write.shutdown().await.unwrap();
        gateway.await.unwrap().unwrap();
    }
}
