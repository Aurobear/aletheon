//! Aletheon typed Gateway client (CGP-02 owner seam).
//!
//! One client unifies request/response/event correlation, bounded framing,
//! snapshot+cursor subscription and typed failure.  The transport trait keeps
//! the contract testable over an in-memory pipe; the real socket transport is
//! supplied by the composition root.  This crate does not mint canonical core
//! IDs, derive effective policy, or infer terminal — terminal only ever comes
//! from a typed `TurnSettlement` event (gateway-protocol).

mod legacy;
pub use legacy::{LegacyJsonRpcClient, LegacyJsonRpcEventStream, LegacyProtocolClient};

use async_trait::async_trait;
use crate::protocol::{
    Command, Cursor, Event, ProtocolError, Query, SessionRef, SessionSnapshotQuery, TurnRef,
    WireRequest, WireRequestBody, WireResponse, WireResponseBody, PROTOCOL_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{sleep, Duration};

/// Default maximum JSON-line frame accepted by the typed Gateway transport.
/// A client must fail closed on an oversized frame instead of allocating an
/// unbounded buffer for a peer-controlled payload.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Typed outcome of issuing a command.  `Created`/`Resumed` carry the
/// server-assigned opaque reference; the client consumes it, never mints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Created {
        session: SessionRef,
    },
    Resumed {
        session: SessionRef,
    },
    SessionUpdated {
        session: SessionRef,
    },
    Forked {
        session: SessionRef,
    },
    ModelUpdated {
        model: String,
    },
    CollaborationModeUpdated {
        mode: crate::protocol::RequestedCollaborationMode,
    },
    AgentProfileUpdated {
        profile: String,
    },
    Submitted {
        turn: TurnRef,
    },
    WorkspaceRestored {
        outcome: crate::protocol::WorkspaceRestoreOutcome,
    },
    TransactionReviewed {
        outcome: serde_json::Value,
    },
    ExtensionResult {
        result: serde_json::Value,
    },
    Cancelled,
    ApprovalRecorded,
}

/// A bounded, versioned transport for one request/event stream.
#[async_trait]
pub trait GatewayTransport: Send + Sync {
    async fn send_command(&mut self, command: Command) -> Result<CommandOutcome, ProtocolError>;
    async fn send_query(&mut self, query: Query) -> Result<serde_json::Value, ProtocolError>;
    async fn next_event(&mut self) -> Result<Event, ProtocolError>;
}

/// Transport capability for reconnecting without changing the typed client
/// or inventing a new session/turn identity.  Reconnection only replaces the
/// underlying stream; the caller-owned subscription cursor remains intact.
#[async_trait]
pub trait ReconnectableGatewayTransport: GatewayTransport {
    async fn reconnect(&mut self) -> Result<(), ProtocolError>;
}

/// Bounded reconnect policy for a presentation client.  Runtime/session
/// authority is never recreated here; after reconnect the subscription uses
/// its last server-issued cursor to recover a projection snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub max_attempts: usize,
    pub backoff: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_millis(100),
        }
    }
}

/// Typed Gateway client facade.
pub struct GatewayClient<T: GatewayTransport> {
    transport: T,
}

impl<T: GatewayTransport> GatewayClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub async fn send(&mut self, command: Command) -> Result<CommandOutcome, ProtocolError> {
        self.transport.send_command(command).await
    }

    pub async fn query(&mut self, query: Query) -> Result<serde_json::Value, ProtocolError> {
        self.transport.send_query(query).await
    }

    pub async fn next_event(&mut self) -> Result<Event, ProtocolError> {
        self.transport.next_event().await
    }

    /// Move this client into a cursor-backed session subscription.  The
    /// subscription owns the same typed client and never mints a canonical
    /// SessionId or TurnId.
    pub fn subscribe(self, session: SessionRef) -> SessionSubscription<T> {
        SessionSubscription {
            client: self,
            session,
            cursor: Cursor::origin(),
        }
    }
}

impl<T: ReconnectableGatewayTransport> GatewayClient<T> {
    /// Reconnect the transport with bounded retries.  Request correlation and
    /// subscription cursor state stay above the transport boundary.
    pub async fn reconnect(&mut self, policy: ReconnectPolicy) -> Result<(), ProtocolError> {
        let mut last_error = ProtocolError::ConnectionClosed;
        for attempt in 0..=policy.max_attempts {
            match self.transport.reconnect().await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    last_error = error;
                    if attempt < policy.max_attempts && !policy.backoff.is_zero() {
                        sleep(policy.backoff).await;
                    }
                }
            }
        }
        Err(last_error)
    }
}

/// A typed, cursor-backed subscription for one server-owned session.  The
/// snapshot body remains the existing Gateway projection JSON for backwards
/// compatibility, while the cursor itself is always a typed protocol value.
pub struct SessionSubscription<T: GatewayTransport> {
    client: GatewayClient<T>,
    session: SessionRef,
    cursor: Cursor,
}

impl<T: GatewayTransport> SessionSubscription<T> {
    pub fn session(&self) -> &SessionRef {
        &self.session
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Adopt a cursor returned by the authenticated Gateway projection.  The
    /// client cannot derive or mint it locally; this setter is intentionally
    /// explicit so a stale cursor is visible to the caller.
    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    /// Request the session projection after the last acknowledged cursor.
    /// If the projection includes `cursor` or `next_cursor`, capture it as a
    /// typed cursor for the next reconnect/recovery attempt.
    pub async fn snapshot(&mut self) -> Result<serde_json::Value, ProtocolError> {
        let value = self
            .client
            .query(Query::SessionSnapshot(SessionSnapshotQuery {
                session: self.session.clone(),
                after_cursor: Some(self.cursor.clone()),
            }))
            .await?;
        if let Some(cursor) = cursor_from_projection(&value) {
            self.cursor = cursor;
        }
        Ok(value)
    }

    pub async fn next_event(&mut self) -> Result<Event, ProtocolError> {
        self.client.next_event().await
    }

    pub fn into_client(self) -> GatewayClient<T> {
        self.client
    }
}

impl<T: ReconnectableGatewayTransport> SessionSubscription<T> {
    /// Reconnect and immediately recover the session projection from the
    /// server-issued cursor.  No local replay or terminal inference occurs.
    pub async fn reconnect_and_snapshot(
        &mut self,
        policy: ReconnectPolicy,
    ) -> Result<serde_json::Value, ProtocolError> {
        self.client.reconnect(policy).await?;
        self.snapshot().await
    }
}

fn cursor_from_projection(value: &serde_json::Value) -> Option<Cursor> {
    let object = value.as_object()?;
    for key in ["next_cursor", "cursor"] {
        if let Some(candidate) = object.get(key) {
            if let Ok(cursor) = serde_json::from_value::<Cursor>(candidate.clone()) {
                return Some(cursor);
            }
        }
    }
    None
}

/// Production-capable JSON-line Unix socket transport for the typed Gateway
/// envelope. It accepts interleaved typed events while waiting for a matching
/// response and rejects version/request-correlation mismatches fail-closed.
pub struct UnixSocketTransport {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    path: std::path::PathBuf,
    max_frame_bytes: usize,
    next_request_id: u64,
    pending_events: std::collections::VecDeque<Event>,
}

impl UnixSocketTransport {
    pub async fn connect(path: impl AsRef<std::path::Path>) -> Result<Self, ProtocolError> {
        Self::connect_with_max_frame_bytes(path, DEFAULT_MAX_FRAME_BYTES).await
    }

    pub async fn connect_with_max_frame_bytes(
        path: impl AsRef<std::path::Path>,
        max_frame_bytes: usize,
    ) -> Result<Self, ProtocolError> {
        if max_frame_bytes == 0 {
            return Err(ProtocolError::FrameTooLarge);
        }
        let path = path.as_ref().to_path_buf();
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            path,
            max_frame_bytes,
            next_request_id: 1,
            pending_events: std::collections::VecDeque::new(),
        })
    }

    async fn read_frame(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let mut line = Vec::new();
        loop {
            let (take, done) = {
                let buffer = self
                    .reader
                    .fill_buf()
                    .await
                    .map_err(|_| ProtocolError::ConnectionClosed)?;
                if buffer.is_empty() {
                    return Err(ProtocolError::ConnectionClosed);
                }
                let newline = buffer.iter().position(|byte| *byte == b'\n');
                let take = newline.map_or(buffer.len(), |position| position + 1);
                if line.len().saturating_add(take) > self.max_frame_bytes {
                    return Err(ProtocolError::FrameTooLarge);
                }
                line.extend_from_slice(&buffer[..take]);
                (take, newline.is_some())
            };
            self.reader.consume(take);
            if done {
                break;
            }
        }
        Ok(line)
    }

    async fn request(&mut self, body: WireRequestBody) -> Result<WireResponseBody, ProtocolError> {
        let request_id = format!("gateway-{}", self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: request_id.clone(),
            body,
        };
        let line = serde_json::to_vec(&request).map_err(|_| ProtocolError::UnknownSchema)?;
        if line.len() > self.max_frame_bytes {
            return Err(ProtocolError::FrameTooLarge);
        }
        self.writer
            .write_all(&line)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        self.writer
            .flush()
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        loop {
            let line = self.read_frame().await?;
            let response: WireResponse =
                serde_json::from_slice(&line).map_err(|_| ProtocolError::UnknownSchema)?;
            if response.version != PROTOCOL_VERSION {
                return Err(ProtocolError::VersionMismatch {
                    server: response.version,
                    client: PROTOCOL_VERSION,
                });
            }
            if let WireResponseBody::Event(event) = response.body {
                self.pending_events.push_back(event);
                continue;
            }
            if response.request_id != request_id {
                return Err(ProtocolError::UnknownSchema);
            }
            return Ok(response.body);
        }
    }
}

#[async_trait]
impl GatewayTransport for UnixSocketTransport {
    async fn send_command(&mut self, command: Command) -> Result<CommandOutcome, ProtocolError> {
        match self.request(WireRequestBody::Command(command)).await? {
            WireResponseBody::Command(outcome) => match outcome {
                crate::protocol::CommandOutcome::Created { session } => {
                    Ok(CommandOutcome::Created { session })
                }
                crate::protocol::CommandOutcome::Resumed { session } => {
                    Ok(CommandOutcome::Resumed { session })
                }
                crate::protocol::CommandOutcome::SessionUpdated { session } => {
                    Ok(CommandOutcome::SessionUpdated { session })
                }
                crate::protocol::CommandOutcome::Forked { session } => {
                    Ok(CommandOutcome::Forked { session })
                }
                crate::protocol::CommandOutcome::ModelUpdated { model } => {
                    Ok(CommandOutcome::ModelUpdated { model })
                }
                crate::protocol::CommandOutcome::CollaborationModeUpdated { mode } => {
                    Ok(CommandOutcome::CollaborationModeUpdated { mode })
                }
                crate::protocol::CommandOutcome::AgentProfileUpdated { profile } => {
                    Ok(CommandOutcome::AgentProfileUpdated { profile })
                }
                crate::protocol::CommandOutcome::Submitted { turn } => {
                    Ok(CommandOutcome::Submitted { turn })
                }
                crate::protocol::CommandOutcome::WorkspaceRestored { outcome } => {
                    Ok(CommandOutcome::WorkspaceRestored { outcome })
                }
                crate::protocol::CommandOutcome::TransactionReviewed { outcome } => {
                    Ok(CommandOutcome::TransactionReviewed { outcome })
                }
                crate::protocol::CommandOutcome::ExtensionResult { result } => {
                    Ok(CommandOutcome::ExtensionResult { result })
                }
                crate::protocol::CommandOutcome::Cancelled => Ok(CommandOutcome::Cancelled),
                crate::protocol::CommandOutcome::ApprovalRecorded => {
                    Ok(CommandOutcome::ApprovalRecorded)
                }
            },
            WireResponseBody::Error(error) => Err(error),
            _ => Err(ProtocolError::UnknownSchema),
        }
    }

    async fn send_query(&mut self, query: Query) -> Result<serde_json::Value, ProtocolError> {
        match self.request(WireRequestBody::Query(query)).await? {
            WireResponseBody::Query(value) => Ok(value),
            WireResponseBody::Error(error) => Err(error),
            _ => Err(ProtocolError::UnknownSchema),
        }
    }

    async fn next_event(&mut self) -> Result<Event, ProtocolError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }
        let line = self.read_frame().await?;
        let response: WireResponse =
            serde_json::from_slice(&line).map_err(|_| ProtocolError::UnknownSchema)?;
        if response.version != PROTOCOL_VERSION {
            return Err(ProtocolError::VersionMismatch {
                server: response.version,
                client: PROTOCOL_VERSION,
            });
        }
        match response.body {
            WireResponseBody::Event(event) => Ok(event),
            WireResponseBody::Error(error) => Err(error),
            _ => Err(ProtocolError::UnknownSchema),
        }
    }
}

#[async_trait]
impl ReconnectableGatewayTransport for UnixSocketTransport {
    async fn reconnect(&mut self) -> Result<(), ProtocolError> {
        let stream = UnixStream::connect(&self.path)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        let (reader, writer) = stream.into_split();
        self.reader = BufReader::new(reader);
        self.writer = writer;
        self.pending_events.clear();
        Ok(())
    }
}

/// In-memory transport for contract verification (CGP-02 acceptance: "client
/// contract 可用 in-memory transport 验证").  Echoes typed commands to the
/// outcome and exposes a scripted event stream.
#[derive(Debug, Default)]
pub struct InMemoryTransport {
    session: Option<SessionRef>,
    events: std::vec::IntoIter<Event>,
}

impl InMemoryTransport {
    pub fn new() -> Self {
        Self {
            session: None,
            events: Vec::new().into_iter(),
        }
    }

    pub fn with_events(events: Vec<Event>) -> Self {
        Self {
            session: None,
            events: events.into_iter(),
        }
    }
}

#[async_trait]
impl GatewayTransport for InMemoryTransport {
    async fn send_command(&mut self, command: Command) -> Result<CommandOutcome, ProtocolError> {
        match command {
            Command::CreateSession(_) => {
                let session = SessionRef("session-1".into());
                self.session = Some(session.clone());
                Ok(CommandOutcome::Created { session })
            }
            Command::ResumeSession(ref r) => {
                let session = SessionRef(r.reference.clone());
                self.session = Some(session.clone());
                Ok(CommandOutcome::Resumed { session })
            }
            Command::ClearSession(session) | Command::CompactSession(session) => {
                self.session = Some(session.clone());
                Ok(CommandOutcome::SessionUpdated { session })
            }
            Command::ForkSession(request) => {
                self.session = Some(request.session.clone());
                Ok(CommandOutcome::Forked {
                    session: request.session,
                })
            }
            Command::SetModel(request) => Ok(CommandOutcome::ModelUpdated {
                model: request.model,
            }),
            Command::SetCollaborationMode(request) => {
                Ok(CommandOutcome::CollaborationModeUpdated { mode: request.mode })
            }
            Command::SetAgentProfile(request) => Ok(CommandOutcome::AgentProfileUpdated {
                profile: request.profile,
            }),
            Command::SubmitPrompt(_) => Ok(CommandOutcome::Submitted {
                turn: TurnRef("turn-1".into()),
            }),
            Command::InvokeSkill(_) => Ok(CommandOutcome::Submitted {
                turn: TurnRef("turn-1".into()),
            }),
            Command::RestoreWorkspaceCheckpoint(_) => Ok(CommandOutcome::WorkspaceRestored {
                outcome: crate::protocol::WorkspaceRestoreOutcome::Completed,
            }),
            Command::ExecuteShell(_) => Ok(CommandOutcome::Submitted {
                turn: TurnRef("turn-1".into()),
            }),
            Command::ReviewTransaction(_) => Ok(CommandOutcome::TransactionReviewed {
                outcome: serde_json::json!({"status": "accepted"}),
            }),
            Command::ManageExtension(_) => Ok(CommandOutcome::ExtensionResult {
                result: serde_json::json!({"status": "ok"}),
            }),
            Command::CancelActiveTurn(_) => Ok(CommandOutcome::Cancelled),
            Command::SubmitApproval(_) => Ok(CommandOutcome::ApprovalRecorded),
        }
    }

    async fn send_query(&mut self, query: Query) -> Result<serde_json::Value, ProtocolError> {
        match query {
            Query::SessionSnapshot(q) => Ok(serde_json::json!({
                "session": q.session.0,
                "after": q.after_cursor.map(|c| {
                    serde_json::json!({
                        "sequence": c.sequence,
                        "event_id": c.event_id,
                    })
                }),
            })),
            Query::SessionList => Ok(serde_json::json!({
                "schema_version": ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION,
                "sessions": [],
            })),
            Query::SkillCatalog(_) => Ok(serde_json::json!({"skills": []})),
            Query::ModelCatalog(_) => Ok(serde_json::json!({"models": [], "current": ""})),
            Query::AgentCatalog(_) => Ok(serde_json::json!({"agents": []})),
            Query::AgentProfileCatalog(_) => Ok(serde_json::json!({"profiles": []})),
            Query::MemoryStatus(_) => Ok(serde_json::json!({"memory": {}})),
            Query::MemorySearch(_) => Ok(serde_json::json!({"facts": []})),
            Query::MemorySnapshot(_) => Ok(serde_json::json!({
                "memory_type": "all",
                "content": "# Memory\n\n*(no entries)*",
            })),
            Query::CheckpointList(q) => Ok(serde_json::json!({
                "schema_version": ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
                "session_id": q.session.0,
                "checkpoints": [],
            })),
            Query::TransactionSettlement(_) => Ok(serde_json::json!({"receipt": null})),
        }
    }

    async fn next_event(&mut self) -> Result<Event, ProtocolError> {
        self.events.next().ok_or(ProtocolError::ConnectionClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        RequestSessionCreation, SettlementTerminal, SubmitPromptRequest, TurnSettlement,
    };

    #[tokio::test]
    async fn create_session_returns_opaque_ref_without_client_mint() {
        let mut client = GatewayClient::new(InMemoryTransport::new());
        let outcome = client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: None,
            }))
            .await
            .unwrap();
        match outcome {
            CommandOutcome::Created { session } => assert_eq!(session.0, "session-1"),
            _ => panic!("expected Created"),
        }
    }

    #[tokio::test]
    async fn submit_prompt_returns_turn_ref() {
        let mut client = GatewayClient::new(InMemoryTransport::new());
        let outcome = client
            .send(Command::SubmitPrompt(SubmitPromptRequest {
                session: SessionRef("session-1".into()),
                content: "hello".into(),
                workspace: None,
                requested_target: Default::default(),
                requested_permission: Default::default(),
                required_agent_runtimes: Vec::new(),
                requested_task_kind: None,
            }))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            CommandOutcome::Submitted {
                turn: TurnRef("turn-1".into()),
            }
        );
    }

    #[tokio::test]
    async fn terminal_only_from_typed_settlement_event() {
        let mut client =
            GatewayClient::new(InMemoryTransport::with_events(vec![Event::TurnSettled(
                TurnSettlement {
                    turn: TurnRef("turn-1".into()),
                    terminal: SettlementTerminal::Completed,
                },
            )]));
        match client.next_event().await.unwrap() {
            Event::TurnSettled(s) => assert_eq!(s.terminal, SettlementTerminal::Completed),
            _ => panic!("expected TurnSettled"),
        }
    }

    #[tokio::test]
    async fn event_stream_exhaustion_is_typed_connection_closed() {
        let mut client = GatewayClient::new(InMemoryTransport::new());
        assert_eq!(
            client.next_event().await.unwrap_err(),
            ProtocolError::ConnectionClosed
        );
    }

    #[tokio::test]
    async fn subscription_keeps_server_cursor_and_uses_it_for_snapshot() {
        let mut subscription =
            GatewayClient::new(InMemoryTransport::new()).subscribe(SessionRef("session-1".into()));
        assert_eq!(subscription.cursor(), &crate::protocol::Cursor::origin());
        subscription.set_cursor(crate::protocol::Cursor {
            sequence: 7,
            event_id: Some("event-7".into()),
        });
        let snapshot = subscription.snapshot().await.unwrap();
        assert_eq!(snapshot["after"]["sequence"], 7);
        assert_eq!(snapshot["after"]["event_id"], "event-7");
    }

    struct ReconnectableTransport {
        reconnects: usize,
    }

    #[async_trait]
    impl GatewayTransport for ReconnectableTransport {
        async fn send_command(
            &mut self,
            _command: Command,
        ) -> Result<CommandOutcome, ProtocolError> {
            Ok(CommandOutcome::Cancelled)
        }

        async fn send_query(&mut self, _query: Query) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"cursor": {"sequence": 9}}))
        }

        async fn next_event(&mut self) -> Result<Event, ProtocolError> {
            Err(ProtocolError::ConnectionClosed)
        }
    }

    #[async_trait]
    impl ReconnectableGatewayTransport for ReconnectableTransport {
        async fn reconnect(&mut self) -> Result<(), ProtocolError> {
            self.reconnects += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn reconnect_recovers_subscription_from_typed_cursor() {
        let mut subscription = GatewayClient::new(ReconnectableTransport { reconnects: 0 })
            .subscribe(SessionRef("session-1".into()));
        let value = subscription
            .reconnect_and_snapshot(ReconnectPolicy {
                max_attempts: 0,
                backoff: Duration::ZERO,
            })
            .await
            .unwrap();
        assert_eq!(subscription.cursor().sequence, 9);
        assert_eq!(value["cursor"]["sequence"], 9);
        let mut client = subscription.into_client();
        client.reconnect(ReconnectPolicy::default()).await.unwrap();
    }

    #[tokio::test]
    async fn unix_socket_transport_roundtrips_typed_command_and_correlation() {
        use crate::protocol::{WireResponse, WireResponseBody};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gateway.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = Vec::new();
            reader.read_until(b'\n', &mut line).await.unwrap();
            let request: WireRequest = serde_json::from_slice(&line).unwrap();
            let progress = WireResponse {
                version: PROTOCOL_VERSION,
                request_id: "out-of-band".into(),
                body: WireResponseBody::Event(crate::protocol::Event::Progress(
                    crate::protocol::RuntimeProgressEvent {
                        kind: "client_event".into(),
                        payload: serde_json::json!({"text": "progress"}),
                    },
                )),
            };
            writer
                .write_all(serde_json::to_string(&progress).unwrap().as_bytes())
                .await
                .unwrap();
            writer.write_all(b"\n").await.unwrap();
            let response = WireResponse {
                version: PROTOCOL_VERSION,
                request_id: request.request_id,
                body: WireResponseBody::Command(crate::protocol::CommandOutcome::Created {
                    session: SessionRef("server-session".into()),
                }),
            };
            writer
                .write_all(serde_json::to_string(&response).unwrap().as_bytes())
                .await
                .unwrap();
            writer.write_all(b"\n").await.unwrap();
        });
        let mut client = GatewayClient::new(UnixSocketTransport::connect(&path).await.unwrap());
        let outcome = client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: None,
            }))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            CommandOutcome::Created {
                session: SessionRef("server-session".into())
            }
        );
        assert!(matches!(
            client.next_event().await.unwrap(),
            crate::protocol::Event::Progress(_)
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unix_socket_transport_rejects_oversized_frame_before_json_decode() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bounded-gateway.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut request = Vec::new();
            reader.read_until(b'\n', &mut request).await.unwrap();
            writer.write_all(&vec![b'x'; 256]).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
        });

        let mut client = GatewayClient::new(
            UnixSocketTransport::connect_with_max_frame_bytes(&path, 256)
                .await
                .unwrap(),
        );
        let error = client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(error, ProtocolError::FrameTooLarge);
        server.await.unwrap();
    }
}
