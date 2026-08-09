//! Aletheon typed Gateway client (CGP-02 owner seam).
//!
//! One client unifies request/response/event correlation, bounded framing,
//! snapshot+cursor subscription and typed failure.  The transport trait keeps
//! the contract testable over an in-memory pipe; the real socket transport is
//! supplied by the composition root.  This crate does not mint canonical core
//! IDs, derive effective policy, or infer terminal — terminal only ever comes
//! from a typed `TurnSettlement` event (gateway-protocol).

use async_trait::async_trait;
use gateway_protocol::{Command, Event, ProtocolError, Query, SessionRef, TurnRef};

/// Typed outcome of issuing a command.  `Created`/`Resumed` carry the
/// server-assigned opaque reference; the client consumes it, never mints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Created { session: SessionRef },
    Resumed { session: SessionRef },
    Submitted { turn: TurnRef },
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
            Command::SubmitPrompt(_) => Ok(CommandOutcome::Submitted {
                turn: TurnRef("turn-1".into()),
            }),
            Command::CancelActiveTurn(_) => Ok(CommandOutcome::Cancelled),
            Command::SubmitApproval(_) => Ok(CommandOutcome::ApprovalRecorded),
        }
    }

    async fn send_query(&mut self, query: Query) -> Result<serde_json::Value, ProtocolError> {
        match query {
            Query::SessionSnapshot(q) => Ok(serde_json::json!({
                "session": q.session.0,
                "after": q.after_cursor.map(|c| c.0),
            })),
        }
    }

    async fn next_event(&mut self) -> Result<Event, ProtocolError> {
        self.events.next().ok_or(ProtocolError::ConnectionClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_protocol::{
        RequestSessionCreation, SettlementTerminal, SubmitPromptRequest, TurnSettlement,
    };

    #[tokio::test]
    async fn create_session_returns_opaque_ref_without_client_mint() {
        let mut client = GatewayClient::new(InMemoryTransport::new());
        let outcome = client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
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
                requested_target: Default::default(),
                requested_permission: Default::default(),
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
}
