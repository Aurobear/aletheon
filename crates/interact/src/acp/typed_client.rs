//! CGP-05 ACP typed-client seam (Agent Kernel V2).
//!
//! ACP translation + connection-local correlation over the typed
//! `gateway-client` instead of raw JSON-RPC business methods.  Per runbook
//! PR-A this seam is additive and **not switched** — the legacy
//! `ExecutiveAcpBackend` remains authoritative until the CGP-05 cutover.
//! This adapter only translates ACP DTOs to typed Gateway commands; it never
//! executes business logic and never mints a Session/Turn ID.

use async_trait::async_trait;
use gateway_client::{CommandOutcome, GatewayClient, GatewayTransport};
use gateway_protocol::{
    CancelActiveTurn, Command, RequestSessionCreation, ResumeSessionReference, SessionRef,
    SubmitPromptRequest, TurnRef,
};

/// Typed ACP session create/prompt/cancel/recover translation to Gateway
/// commands.  Holds a typed client; never raw JSON-RPC business methods.
pub struct AcpTypedClient<T: GatewayTransport> {
    client: GatewayClient<T>,
}

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
                requested_target: Default::default(),
                requested_permission: Default::default(),
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
    ) -> Result<CommandOutcome, gateway_protocol::ProtocolError> {
        Err(gateway_protocol::ProtocolError::ConnectionClosed)
    }
    async fn send_query(
        &mut self,
        _query: gateway_protocol::Query,
    ) -> Result<serde_json::Value, gateway_protocol::ProtocolError> {
        Err(gateway_protocol::ProtocolError::ConnectionClosed)
    }
    async fn next_event(
        &mut self,
    ) -> Result<gateway_protocol::Event, gateway_protocol::ProtocolError> {
        Err(gateway_protocol::ProtocolError::ConnectionClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_client::InMemoryTransport;

    #[tokio::test]
    async fn typed_aclient_create_returns_server_ref() {
        let mut client = AcpTypedClient::new(GatewayClient::new(InMemoryTransport::new()));
        let session = client.create_session().await.unwrap();
        assert_eq!(session.0, "session-1");
    }
}
