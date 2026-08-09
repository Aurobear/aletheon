//! CGP-03 typed Gateway route handlers (Agent Kernel V2).
//!
//! One-way typed handlers for Session/Turn/Approval, built on
//! `gateway-protocol` typed commands/queries/events.  Each handler only calls
//! an Application/Runtime trait — no Kernel/domain store/concrete adapter
//! import.  `LegacyJsonRpcAdapter` (in the legacy handler path) only
//! translates; it never executes business logic.  This seam is additive: the
//! legacy daemon handlers remain authoritative until the CGP-03 route cutover.

use async_trait::async_trait;
use gateway_protocol::{
    CancelActiveTurn, Command, ProtocolError, RequestSessionCreation, ResumeSessionReference,
    SessionRef, SubmitApprovalChoice, SubmitPromptRequest, TurnRef,
};
use std::sync::Arc;

/// Re-export to keep the typed surface self-contained (the unused alias is
/// deliberate: gateway-protocol does not yet expose an outcome type here).
#[allow(unused_imports)]
pub use gateway_protocol::Command as TypedCommand;

/// The Application trait a typed route handler may call.  Implemented by the
/// Application facade over Runtime ports; the handler never touches a
/// repository or a concrete adapter.
#[async_trait]
pub trait TypedApplicationPort: Send + Sync {
    async fn create_session(&self, hint: Option<String>) -> Result<SessionRef, ProtocolError>;
    async fn resume_session(&self, reference: String) -> Result<SessionRef, ProtocolError>;
    async fn submit_prompt(
        &self,
        session: SessionRef,
        content: String,
    ) -> Result<TurnRef, ProtocolError>;
    async fn cancel_turn(&self, session: SessionRef) -> Result<(), ProtocolError>;
    async fn submit_approval(
        &self,
        session: SessionRef,
        choice: String,
        approved: bool,
    ) -> Result<(), ProtocolError>;
}

/// Typed route handler for the Session/Turn/Approval command families.  Calls
/// only `TypedApplicationPort`.  No Kernel/domain store/concrete adapter
/// import anywhere in this module.
pub struct TypedRouteHandler {
    application: Arc<dyn TypedApplicationPort>,
}

impl TypedRouteHandler {
    pub fn new(application: Arc<dyn TypedApplicationPort>) -> Self {
        Self { application }
    }

    /// Dispatch a typed Gateway command to the Application port.  This is the
    /// one-way adapter: it translates typed commands to port calls and typed
    /// results back — it never executes business logic itself.
    pub async fn dispatch(&self, command: Command) -> Result<CommandOutcome, ProtocolError> {
        match command {
            Command::CreateSession(RequestSessionCreation { principal_hint }) => {
                let session = self.application.create_session(principal_hint).await?;
                Ok(CommandOutcome::Session(session))
            }
            Command::ResumeSession(ResumeSessionReference { reference }) => {
                let session = self.application.resume_session(reference).await?;
                Ok(CommandOutcome::Session(session))
            }
            Command::SubmitPrompt(SubmitPromptRequest {
                session, content, ..
            }) => {
                let turn = self.application.submit_prompt(session, content).await?;
                Ok(CommandOutcome::Turn(turn))
            }
            Command::CancelActiveTurn(CancelActiveTurn { session }) => {
                self.application.cancel_turn(session).await?;
                Ok(CommandOutcome::Ok)
            }
            Command::SubmitApproval(SubmitApprovalChoice {
                session,
                choice_id,
                approved,
            }) => {
                self.application
                    .submit_approval(session, choice_id, approved)
                    .await?;
                Ok(CommandOutcome::Ok)
            }
        }
    }
}

/// Typed command outcome returned to the Gateway client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Session(SessionRef),
    Turn(TurnRef),
    Ok,
}

/// Compatibility adapter: a legacy JSON-RPC request is translated into a typed
/// Gateway command, nothing more.  Business logic never lives here.
pub struct LegacyJsonRpcAdapter {
    inner: TypedRouteHandler,
}

impl LegacyJsonRpcAdapter {
    pub fn new(inner: TypedRouteHandler) -> Self {
        Self { inner }
    }

    /// Translate a legacy method name + JSON body into a typed command, then
    /// dispatch.  Unknown methods fail closed with a typed error.
    pub async fn translate_and_dispatch(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<CommandOutcome, ProtocolError> {
        let command = match method {
            "new_session" | "session.create" => Command::CreateSession(RequestSessionCreation {
                principal_hint: body
                    .get("principal")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            }),
            "resume" | "session.resume" => Command::ResumeSession(ResumeSessionReference {
                reference: body
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }),
            "prompt" | "turn.start" => Command::SubmitPrompt(SubmitPromptRequest {
                session: SessionRef(
                    body.get("session")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                content: body
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                requested_target: Default::default(),
                requested_permission: Default::default(),
            }),
            "cancel" | "turn.cancel" => Command::CancelActiveTurn(CancelActiveTurn {
                session: SessionRef(
                    body.get("session")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
            }),
            "approve" | "approval.approve" => Command::SubmitApproval(SubmitApprovalChoice {
                session: SessionRef(
                    body.get("session")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                choice_id: body
                    .get("choice_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                approved: true,
            }),
            _ => return Err(ProtocolError::UnknownSchema),
        };
        self.inner.dispatch(command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeApp {
        created: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl TypedApplicationPort for FakeApp {
        async fn create_session(&self, hint: Option<String>) -> Result<SessionRef, ProtocolError> {
            self.created.lock().unwrap().push(hint);
            Ok(SessionRef("sess-1".into()))
        }
        async fn resume_session(&self, reference: String) -> Result<SessionRef, ProtocolError> {
            Ok(SessionRef(reference))
        }
        async fn submit_prompt(
            &self,
            _s: SessionRef,
            _c: String,
        ) -> Result<TurnRef, ProtocolError> {
            Ok(TurnRef("turn-1".into()))
        }
        async fn cancel_turn(&self, _s: SessionRef) -> Result<(), ProtocolError> {
            Ok(())
        }
        async fn submit_approval(
            &self,
            _s: SessionRef,
            _choice: String,
            _approved: bool,
        ) -> Result<(), ProtocolError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn typed_handler_creates_session_via_application_port() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app.clone());
        let outcome = handler
            .dispatch(Command::CreateSession(RequestSessionCreation {
                principal_hint: Some("alice".into()),
            }))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            CommandOutcome::Session(SessionRef("sess-1".into()))
        );
    }

    #[tokio::test]
    async fn legacy_adapter_translates_and_dispatches() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let adapter = LegacyJsonRpcAdapter::new(TypedRouteHandler::new(app));
        let outcome = adapter
            .translate_and_dispatch("new_session", serde_json::json!({"principal": "bob"}))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            CommandOutcome::Session(SessionRef("sess-1".into()))
        );
    }
}
