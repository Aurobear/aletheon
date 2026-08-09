//! Application use cases and the typed facade (APX-01).
//!
//! `ApplicationFacade` exposes the Session/Turn/Delegate use cases.  It holds
//! a `runtime::RuntimeCommandPort` / `RuntimeQueryPort` and forwards typed
//! commands — it never owns a repository and never mints a core ID.  The
//! in-memory fake Runtime port verifies command forwarding (APX-01 acceptance).

use crate::error::ApplicationError;
use async_trait::async_trait;
use runtime::{RuntimeCommandPort, RuntimeQueryPort};

/// Create a session by forwarding a typed `runtime::CreateSessionCommand`.
pub struct CreateSession;

/// Resume a session from a reference string (opaque; Runtime resolves it).
pub struct ResumeSession;

/// Fork a session (Runtime assigns the child identity).
pub struct ForkSession;

/// List sessions.
pub struct ListSessions;

/// Get a single session snapshot.
pub struct GetSession;

/// Delete a session.
pub struct DeleteSession;

/// The Application facade.  Implemented over runtime ports; no repository,
/// no core-ID mint.
#[async_trait]
pub trait ApplicationFacade: Send + Sync {
    async fn create_session(
        &self,
        principal_hint: Option<String>,
    ) -> Result<runtime::SessionId, ApplicationError>;

    async fn resume_session(
        &self,
        reference: String,
    ) -> Result<runtime::SessionId, ApplicationError>;

    async fn list_sessions(&self) -> Result<Vec<runtime::SessionId>, ApplicationError>;

    async fn get_session(
        &self,
        session: &runtime::SessionId,
    ) -> Result<serde_json::Value, ApplicationError>;
}

/// Application facade over the Runtime ports.
pub struct DefaultApplicationFacade {
    commands: std::sync::Arc<dyn RuntimeCommandPort>,
    queries: std::sync::Arc<dyn RuntimeQueryPort>,
}

impl DefaultApplicationFacade {
    pub fn new(
        commands: std::sync::Arc<dyn RuntimeCommandPort>,
        queries: std::sync::Arc<dyn RuntimeQueryPort>,
    ) -> Self {
        Self { commands, queries }
    }
}

#[async_trait]
impl ApplicationFacade for DefaultApplicationFacade {
    async fn create_session(
        &self,
        principal_hint: Option<String>,
    ) -> Result<runtime::SessionId, ApplicationError> {
        let receipt = self
            .commands
            .dispatch(runtime::RuntimeCommand::CreateSession(
                runtime::CreateSessionCommand {
                    correlation: None,
                    principal_hint,
                },
            ))
            .await
            .map_err(|e| ApplicationError::RuntimeRejected(e.to_string()))?;
        receipt
            .session
            .ok_or(ApplicationError::InvalidSessionReference)
    }

    async fn resume_session(
        &self,
        reference: String,
    ) -> Result<runtime::SessionId, ApplicationError> {
        let receipt = self
            .commands
            .dispatch(runtime::RuntimeCommand::ResumeSession(
                runtime::ResumeSessionCommand {
                    session_reference: reference,
                },
            ))
            .await
            .map_err(|e| ApplicationError::RuntimeRejected(e.to_string()))?;
        receipt
            .session
            .ok_or(ApplicationError::InvalidSessionReference)
    }

    async fn list_sessions(&self) -> Result<Vec<runtime::SessionId>, ApplicationError> {
        // Minimal list: query a snapshot; real listing arrives with RA-03.
        Ok(vec![])
    }

    async fn get_session(
        &self,
        session: &runtime::SessionId,
    ) -> Result<serde_json::Value, ApplicationError> {
        self.queries
            .query(runtime::RuntimeQuery::SessionSnapshot(
                runtime::SessionSnapshotQuery {
                    session: session.clone(),
                    after_cursor: None,
                },
            ))
            .await
            .map_err(|e| ApplicationError::RuntimeRejected(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory fake Runtime command port (APX-01 acceptance: verify command
    /// forwarding with a fake Runtime port; no repository, no writer).
    #[derive(Default)]
    struct FakeRuntime {
        created: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl RuntimeCommandPort for FakeRuntime {
        async fn dispatch(
            &self,
            command: runtime::RuntimeCommand,
        ) -> Result<runtime::CommandReceipt, runtime::RuntimeError> {
            match command {
                runtime::RuntimeCommand::CreateSession(_) => {
                    let id = runtime::SessionId("sess-fake-1".into());
                    self.created.lock().unwrap().push(id.0.clone());
                    Ok(runtime::CommandReceipt {
                        session: Some(id),
                        turn: None,
                        agent_run: None,
                        generation: None,
                    })
                }
                runtime::RuntimeCommand::ResumeSession(_) => Ok(runtime::CommandReceipt {
                    session: Some(runtime::SessionId("sess-resumed".into())),
                    turn: None,
                    agent_run: None,
                    generation: None,
                }),
                _ => Err(runtime::RuntimeError::Internal),
            }
        }
    }

    #[async_trait]
    impl RuntimeQueryPort for FakeRuntime {
        async fn query(
            &self,
            query: runtime::RuntimeQuery,
        ) -> Result<serde_json::Value, runtime::RuntimeError> {
            match query {
                runtime::RuntimeQuery::SessionSnapshot(_) => Ok(serde_json::json!({"ok": true})),
                _ => Err(runtime::RuntimeError::Internal),
            }
        }
    }

    #[tokio::test]
    async fn create_session_forwards_to_runtime_and_returns_receipt() {
        let fake: std::sync::Arc<dyn RuntimeCommandPort> =
            std::sync::Arc::new(FakeRuntime::default());
        let fake_q: std::sync::Arc<dyn RuntimeQueryPort> =
            std::sync::Arc::new(FakeRuntime::default());
        let facade = DefaultApplicationFacade::new(fake, fake_q);
        let session = facade.create_session(Some("alice".into())).await.unwrap();
        assert_eq!(session.0, "sess-fake-1");
    }

    #[tokio::test]
    async fn resume_session_returns_runtime_assigned_id() {
        let fake: std::sync::Arc<dyn RuntimeCommandPort> =
            std::sync::Arc::new(FakeRuntime::default());
        let fake_q: std::sync::Arc<dyn RuntimeQueryPort> =
            std::sync::Arc::new(FakeRuntime::default());
        let facade = DefaultApplicationFacade::new(fake, fake_q);
        let session = facade.resume_session("legacy-ref".into()).await.unwrap();
        assert_eq!(session.0, "sess-resumed");
    }
}
