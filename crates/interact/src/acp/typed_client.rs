//! CGP-05 ACP typed-client seam (Agent Kernel V2).
//!
//! ACP translation + connection-local correlation over the typed
//! `gateway::client` module instead of raw JSON-RPC business methods. The production
//! ACP adapter below is intentionally transport-only: it never opens a store,
//! starts Runtime, executes a turn, or mints a Session/Turn ID.

use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use gateway::client::{CommandOutcome, GatewayClient, GatewayTransport, UnixSocketTransport};
use gateway::protocol::{
    CancelActiveTurn, Command, Cursor, Query, RequestSessionCreation, ResumeSessionReference,
    SessionRef, SessionSnapshotQuery, SubmitPromptRequest, TurnRef,
};
use tokio::sync::Mutex;

use super::{AcpBackend, AcpError, AcpEventSource, AcpSessionEvent, CreatedAcpSession};
use ::contracts::{
    protocol::client::{EventCursor, SessionReadSnapshot, UiSnapshot},
    PrincipalContext, ThreadId,
};

/// Typed ACP session create/prompt/cancel/recover translation to Gateway
/// commands.  Holds a typed client; never raw JSON-RPC business methods.
///
#[allow(dead_code)]
pub struct AcpTypedClient<T: GatewayTransport> {
    client: GatewayClient<T>,
}

pub type SharedGatewayClient = Arc<Mutex<GatewayClient<UnixSocketTransport>>>;

/// ACP backend backed by the official typed Gateway socket.
///
/// The ACP connection still keeps its own correlation table, but all
/// Session/Turn authority remains on the daemon.  `PrincipalContext` and
/// `cwd` are used only by the outer ACP adapter for local framing/authorization;
/// the Gateway authenticates the Unix peer and computes effective policy.
pub struct GatewayAcpBackend {
    client: SharedGatewayClient,
    active_session: Arc<Mutex<Option<String>>>,
    active_workspace: Arc<Mutex<Option<String>>>,
}

impl GatewayAcpBackend {
    pub async fn connect(path: impl AsRef<Path>) -> Result<(Self, GatewayAcpEvents), AcpError> {
        let client = GatewayClient::new(
            UnixSocketTransport::connect(path)
                .await
                .map_err(|error| AcpError::Backend(error.to_string()))?,
        );
        let client = Arc::new(Mutex::new(client));
        let active_session = Arc::new(Mutex::new(None));
        let active_workspace = Arc::new(Mutex::new(None));
        Ok((
            Self {
                client: client.clone(),
                active_session: active_session.clone(),
                active_workspace: active_workspace.clone(),
            },
            GatewayAcpEvents {
                client,
                active_session,
                last_cursor: HashMap::new(),
            },
        ))
    }
}

#[async_trait]
impl AcpBackend for GatewayAcpBackend {
    async fn create_session(
        &self,
        _principal: &PrincipalContext,
        _cwd: &Path,
    ) -> Result<CreatedAcpSession, AcpError> {
        let outcome = self
            .client
            .lock()
            .await
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: Some(_cwd.to_string_lossy().into_owned()),
            }))
            .await
            .map_err(|error| AcpError::Backend(error.to_string()))?;
        let CommandOutcome::Created { session } = outcome else {
            return Err(AcpError::Backend(
                "Gateway did not return session receipt".into(),
            ));
        };
        *self.active_session.lock().await = Some(session.0.clone());
        *self.active_workspace.lock().await = Some(_cwd.to_string_lossy().into_owned());
        // ThreadId is only the existing ACP lookup-key field. The value is
        // copied from the server-returned opaque reference; no client UUID is
        // minted and it is never used as a Runtime authority key.
        Ok(CreatedAcpSession {
            thread_id: ThreadId(session.0.clone()),
            session_id: session.0,
        })
    }

    async fn submit_prompt(
        &self,
        _principal: &PrincipalContext,
        session_id: &str,
        _thread_id: &ThreadId,
        text: &str,
    ) -> Result<(), AcpError> {
        let outcome = self
            .client
            .lock()
            .await
            .send(Command::SubmitPrompt(SubmitPromptRequest {
                session: SessionRef(session_id.to_owned()),
                content: text.to_owned(),
                workspace: self.active_workspace.lock().await.clone(),
                requested_target: Default::default(),
                requested_permission: Default::default(),
                required_agent_runtimes: Vec::new(),
                requested_task_kind: None,
            }))
            .await
            .map_err(|error| AcpError::Backend(error.to_string()))?;
        if !matches!(outcome, CommandOutcome::Submitted { .. }) {
            return Err(AcpError::Backend(
                "Gateway did not return turn receipt".into(),
            ));
        }
        *self.active_session.lock().await = Some(session_id.to_owned());
        Ok(())
    }

    async fn cancel_turn(
        &self,
        _principal: &PrincipalContext,
        session_id: &str,
        _thread_id: &ThreadId,
    ) -> Result<(), AcpError> {
        self.client
            .lock()
            .await
            .send(Command::CancelActiveTurn(CancelActiveTurn {
                session: SessionRef(session_id.to_owned()),
            }))
            .await
            .map_err(|error| AcpError::Backend(error.to_string()))?;
        Ok(())
    }
}

/// Projection-only ACP event source. Until the Gateway server publishes a
/// dedicated event stream on the typed socket, it polls the authoritative
/// snapshot cursor. This keeps recovery projection-based and avoids treating
/// transport EOF or prompt completion text as terminal authority.
pub struct GatewayAcpEvents {
    client: SharedGatewayClient,
    active_session: Arc<Mutex<Option<String>>>,
    last_cursor: HashMap<String, EventCursor>,
}

impl GatewayAcpEvents {
    async fn page(
        &self,
        session: &str,
        after: Option<&EventCursor>,
    ) -> Result<
        (
            SessionReadSnapshot,
            Vec<::contracts::protocol::client::ClientEvent>,
            EventCursor,
        ),
        AcpError,
    > {
        let value = self
            .client
            .lock()
            .await
            .query(Query::SessionSnapshot(SessionSnapshotQuery {
                session: SessionRef(session.to_owned()),
                after_cursor: after.map(|cursor| Cursor {
                    sequence: cursor.sequence,
                    event_id: cursor.event_id.clone(),
                }),
            }))
            .await
            .map_err(|error| AcpError::Backend(error.to_string()))?;
        let snapshot = value
            .get("snapshot")
            .cloned()
            .or_else(|| value.get(0).cloned())
            .ok_or_else(|| AcpError::Backend("Gateway snapshot missing payload".into()))?;
        let snapshot: SessionReadSnapshot = serde_json::from_value(snapshot)
            .map_err(|error| AcpError::Backend(format!("invalid Gateway snapshot: {error}")))?;
        let events = value
            .get("events")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| AcpError::Backend(format!("invalid Gateway event page: {error}")))?
            .unwrap_or_default();
        let next = value
            .get("next")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| AcpError::Backend(format!("invalid Gateway next cursor: {error}")))?
            .unwrap_or_else(|| snapshot.through.clone());
        Ok((snapshot, events, next))
    }

    fn snapshot_event(session: &str, snapshot: &SessionReadSnapshot) -> AcpSessionEvent {
        AcpSessionEvent {
            session_id: session.to_owned(),
            event: ::contracts::protocol::client::ClientEvent::Snapshot(UiSnapshot {
                session_id: snapshot.session.id.clone(),
                cursor: snapshot.through.clone(),
                provider: None,
                model: None,
                items: snapshot.items.clone(),
                approvals: Vec::new(),
                agents: Vec::new(),
            }),
        }
    }
}

#[async_trait]
impl AcpEventSource for GatewayAcpEvents {
    async fn next_event(&mut self) -> Result<Option<AcpSessionEvent>, AcpError> {
        loop {
            let Some(session) = self.active_session.lock().await.clone() else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let previous = self.last_cursor.get(&session).cloned();
            let (snapshot, events, next) = self.page(&session, previous.as_ref()).await?;
            if let Some(event) = events.into_iter().next() {
                let event_cursor = match &event {
                    ::contracts::protocol::client::ClientEvent::Item(item) => item.cursor.clone(),
                    ::contracts::protocol::client::ClientEvent::ApprovalRequested {
                        cursor,
                        ..
                    } => cursor.clone(),
                    ::contracts::protocol::client::ClientEvent::Reconnected(cursor) => {
                        cursor.clone()
                    }
                    ::contracts::protocol::client::ClientEvent::Failed {
                        cursor: Some(cursor),
                        ..
                    } => cursor.clone(),
                    _ => next,
                };
                self.last_cursor.insert(session.clone(), event_cursor);
                return Ok(Some(AcpSessionEvent {
                    session_id: session,
                    event,
                }));
            }
            if previous
                .as_ref()
                .map_or(true, |cursor| snapshot.through != *cursor)
            {
                self.last_cursor.insert(session.clone(), next);
                return Ok(Some(Self::snapshot_event(&session, &snapshot)));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn recover(
        &mut self,
        session_id: &str,
        cursor: &EventCursor,
    ) -> Result<Vec<AcpSessionEvent>, AcpError> {
        let (snapshot, events, next) = self.page(session_id, Some(cursor)).await?;
        self.last_cursor.insert(session_id.to_owned(), next);
        if events.is_empty() {
            Ok(vec![Self::snapshot_event(session_id, &snapshot)])
        } else {
            Ok(events
                .into_iter()
                .map(|event| AcpSessionEvent {
                    session_id: session_id.to_owned(),
                    event,
                })
                .collect())
        }
    }
}

#[allow(dead_code)]
impl<T: GatewayTransport> AcpTypedClient<T> {
    pub fn new(client: GatewayClient<T>) -> Self {
        Self { client }
    }

    /// Translate an ACP create-session into a typed Gateway command.  Returns
    /// the server-assigned opaque SessionRef (never a client-minted ID).
    pub async fn create_session(&mut self) -> Result<SessionRef, String> {
        match self
            .client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: None,
            }))
            .await
        {
            Ok(CommandOutcome::Created { session }) => Ok(session),
            Ok(_) => Err("unexpected create outcome".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Translate an ACP resume into a typed Gateway resume reference.
    pub async fn resume_session(&mut self, reference: String) -> Result<SessionRef, String> {
        match self
            .client
            .send(Command::ResumeSession(ResumeSessionReference { reference }))
            .await
        {
            Ok(CommandOutcome::Resumed { session }) => Ok(session),
            Ok(_) => Err("unexpected resume outcome".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Translate an ACP prompt into a typed SubmitPrompt; returns the TurnRef.
    pub async fn prompt(
        &mut self,
        session: SessionRef,
        content: String,
    ) -> Result<TurnRef, String> {
        match self
            .client
            .send(Command::SubmitPrompt(SubmitPromptRequest {
                session,
                content,
                workspace: None,
                requested_target: Default::default(),
                requested_permission: Default::default(),
                required_agent_runtimes: Vec::new(),
                requested_task_kind: None,
            }))
            .await
        {
            Ok(CommandOutcome::Submitted { turn }) => Ok(turn),
            Ok(_) => Err("unexpected submit outcome".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Translate an ACP cancel into a typed CancelActiveTurn.
    pub async fn cancel(&mut self, session: SessionRef) -> Result<(), String> {
        self.client
            .send(Command::CancelActiveTurn(CancelActiveTurn { session }))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Dummy transport for the seam (no socket; the real socket transport is
/// supplied by the composition root at CGP-05 PR-C).
#[derive(Default)]
pub struct PendingTransport;

#[async_trait]
impl GatewayTransport for PendingTransport {
    async fn send_command(
        &mut self,
        _command: Command,
    ) -> Result<CommandOutcome, gateway::protocol::ProtocolError> {
        Err(gateway::protocol::ProtocolError::ConnectionClosed)
    }
    async fn send_query(
        &mut self,
        _query: gateway::protocol::Query,
    ) -> Result<serde_json::Value, gateway::protocol::ProtocolError> {
        Err(gateway::protocol::ProtocolError::ConnectionClosed)
    }
    async fn next_event(
        &mut self,
    ) -> Result<gateway::protocol::Event, gateway::protocol::ProtocolError> {
        Err(gateway::protocol::ProtocolError::ConnectionClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway::client::InMemoryTransport;

    #[tokio::test]
    async fn typed_aclient_create_returns_server_ref() {
        let mut client = AcpTypedClient::new(GatewayClient::new(InMemoryTransport::new()));
        let session = client.create_session().await.unwrap();
        assert_eq!(session.0, "session-1");
    }
}
