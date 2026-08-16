//! CGP-03 typed Gateway route handlers (Agent Kernel V2).
//!
//! One-way typed handlers for Session/Turn/Approval, built on
//! `gateway-protocol` typed commands/queries/events.  Each handler only calls
//! an Application/Runtime trait — no Kernel/domain store/concrete adapter
//! import.  `LegacyJsonRpcAdapter` (in the legacy handler path) only
//! translates; it never executes business logic.  This seam is additive: the
//! legacy daemon handlers remain authoritative until the CGP-03 route cutover.

use crate::protocol::{
    CancelActiveTurn, Command, ExecuteShellRequest, ExtensionRequest, ForkSessionRequest,
    ProtocolError, Query, RequestSessionCreation, RequestedCollaborationMode,
    RequestedExecutionTarget, RequestedPermissionMode, RestoreWorkspaceCheckpoint,
    ResumeSessionReference, ReviewTransactionRequest, SessionRef, SetAgentProfileRequest,
    SetCollaborationModeRequest, SetModelRequest, SkillInvokeRequest, SubmitApprovalChoice,
    SubmitPromptRequest, TurnRef, WireRequest, WireRequestBody, WireResponse, WireResponseBody,
    WorkspaceRestoreOutcome, PROTOCOL_VERSION,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Re-export to keep the typed surface self-contained (the unused alias is
/// deliberate: gateway-protocol does not yet expose an outcome type here).
#[allow(unused_imports)]
pub use crate::protocol::Command as TypedCommand;

/// The Application trait a typed route handler may call.  Implemented by the
/// Application facade over Runtime ports; the handler never touches a
/// repository or a concrete adapter.
#[async_trait]
pub trait TypedApplicationPort: Send + Sync {
    async fn create_session(
        &self,
        hint: Option<String>,
        workspace: Option<String>,
    ) -> Result<SessionRef, ProtocolError>;
    async fn resume_session(&self, reference: String) -> Result<SessionRef, ProtocolError>;
    async fn clear_session(&self, _session: SessionRef) -> Result<SessionRef, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn fork_session(
        &self,
        _session: SessionRef,
        _through_sequence: u64,
    ) -> Result<SessionRef, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn compact_session(&self, _session: SessionRef) -> Result<SessionRef, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn set_model(&self, _model: String) -> Result<String, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn set_collaboration_mode(
        &self,
        _mode: RequestedCollaborationMode,
    ) -> Result<RequestedCollaborationMode, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn set_agent_profile(&self, _profile: String) -> Result<String, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn invoke_skill(&self, _request: SkillInvokeRequest) -> Result<TurnRef, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn restore_workspace_checkpoint(
        &self,
        _request: RestoreWorkspaceCheckpoint,
    ) -> Result<WorkspaceRestoreOutcome, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn execute_shell(&self, _request: ExecuteShellRequest) -> Result<TurnRef, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn review_transaction(
        &self,
        _request: ReviewTransactionRequest,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn manage_extension(
        &self,
        _request: ExtensionRequest,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
    async fn submit_prompt(
        &self,
        session: SessionRef,
        content: String,
        workspace: Option<String>,
        requested_target: RequestedExecutionTarget,
        requested_permission: RequestedPermissionMode,
        required_agent_runtimes: Vec<String>,
        requested_task_kind: Option<String>,
    ) -> Result<TurnRef, ProtocolError>;
    async fn cancel_turn(&self, session: SessionRef) -> Result<(), ProtocolError>;
    async fn submit_approval(
        &self,
        session: SessionRef,
        choice: String,
        approved: bool,
        version: u64,
        reason: Option<String>,
    ) -> Result<(), ProtocolError>;

    /// Read a projection through the application boundary.  The default is
    /// fail-closed so an adapter cannot accidentally expose an unscoped store
    /// query while a route is still being migrated.
    async fn query_session(
        &self,
        _session: SessionRef,
        _after: Option<crate::protocol::Cursor>,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_sessions(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    /// Read the authenticated host skill catalog.  Skill descriptors are
    /// projection data; enabling/invoking a skill remains a separate command
    /// path with host admission.
    async fn query_skill_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_model_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    /// Read the daemon-owned child-Agent projection.  Agent lifecycle and
    /// identity remain Runtime-owned; Gateway only exposes the authenticated
    /// read model to presentation clients.
    async fn query_agent_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_agent_profile_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_memory_status(&self) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_memory_search(
        &self,
        _query: String,
        _session: Option<String>,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_memory_snapshot(
        &self,
        _session: SessionRef,
        _memory_type: String,
        _limit: u16,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_checkpoint_list(
        &self,
        _session: SessionRef,
        _limit: u16,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }

    async fn query_transaction_settlement(
        &self,
        _session: SessionRef,
        _transaction: String,
    ) -> Result<serde_json::Value, ProtocolError> {
        Err(ProtocolError::UnknownSchema)
    }
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
            Command::CreateSession(RequestSessionCreation {
                principal_hint,
                workspace,
            }) => {
                let session = self
                    .application
                    .create_session(principal_hint, workspace)
                    .await?;
                Ok(CommandOutcome::Session(session))
            }
            Command::ResumeSession(ResumeSessionReference { reference }) => {
                let session = self.application.resume_session(reference).await?;
                Ok(CommandOutcome::Session(session))
            }
            Command::ClearSession(session) => {
                let session = self.application.clear_session(session).await?;
                Ok(CommandOutcome::Updated(session))
            }
            Command::ForkSession(ForkSessionRequest {
                session,
                through_sequence,
            }) => {
                let session = self
                    .application
                    .fork_session(session, through_sequence)
                    .await?;
                Ok(CommandOutcome::Forked(session))
            }
            Command::CompactSession(session) => {
                let session = self.application.compact_session(session).await?;
                Ok(CommandOutcome::Updated(session))
            }
            Command::SetModel(SetModelRequest { model }) => Ok(CommandOutcome::ModelUpdated {
                model: self.application.set_model(model).await?,
            }),
            Command::SetCollaborationMode(SetCollaborationModeRequest { mode }) => {
                Ok(CommandOutcome::CollaborationModeUpdated {
                    mode: self.application.set_collaboration_mode(mode).await?,
                })
            }
            Command::SetAgentProfile(SetAgentProfileRequest { profile }) => {
                Ok(CommandOutcome::AgentProfileUpdated {
                    profile: self.application.set_agent_profile(profile).await?,
                })
            }
            Command::InvokeSkill(request) => Ok(CommandOutcome::Turn(
                self.application.invoke_skill(request).await?,
            )),
            Command::RestoreWorkspaceCheckpoint(request) => Ok(CommandOutcome::WorkspaceRestored {
                outcome: self
                    .application
                    .restore_workspace_checkpoint(request)
                    .await?,
            }),
            Command::ExecuteShell(request) => Ok(CommandOutcome::Turn(
                self.application.execute_shell(request).await?,
            )),
            Command::ReviewTransaction(request) => Ok(CommandOutcome::TransactionReviewed {
                outcome: self.application.review_transaction(request).await?,
            }),
            Command::ManageExtension(request) => Ok(CommandOutcome::ExtensionResult {
                result: self.application.manage_extension(request).await?,
            }),
            Command::SubmitPrompt(SubmitPromptRequest {
                session,
                content,
                workspace,
                requested_target,
                requested_permission,
                required_agent_runtimes,
                requested_task_kind,
            }) => {
                let turn = self
                    .application
                    .submit_prompt(
                        session,
                        content,
                        workspace,
                        requested_target,
                        requested_permission,
                        required_agent_runtimes,
                        requested_task_kind,
                    )
                    .await?;
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
                version,
                reason,
            }) => {
                self.application
                    .submit_approval(session, choice_id, approved, version, reason)
                    .await?;
                Ok(CommandOutcome::Ok)
            }
        }
    }

    /// Decode one versioned wire envelope and return the correlated typed
    /// response. This is deliberately a server-side transport adapter: it
    /// performs no business work beyond calling the application port above.
    pub async fn dispatch_wire(&self, request: WireRequest) -> WireResponse {
        let request_id = request.request_id.clone();
        let body = if request.version != PROTOCOL_VERSION {
            WireResponseBody::Error(ProtocolError::VersionMismatch {
                server: PROTOCOL_VERSION,
                client: request.version,
            })
        } else {
            match request.body {
                WireRequestBody::Command(command) => {
                    let resumed = matches!(&command, Command::ResumeSession(_));
                    let approval = matches!(&command, Command::SubmitApproval(_));
                    match self.dispatch(command).await {
                        Ok(CommandOutcome::Session(session)) => {
                            let outcome = if resumed {
                                crate::protocol::CommandOutcome::Resumed { session }
                            } else {
                                crate::protocol::CommandOutcome::Created { session }
                            };
                            WireResponseBody::Command(outcome)
                        }
                        Ok(CommandOutcome::Updated(session)) => WireResponseBody::Command(
                            crate::protocol::CommandOutcome::SessionUpdated { session },
                        ),
                        Ok(CommandOutcome::Forked(session)) => {
                            WireResponseBody::Command(crate::protocol::CommandOutcome::Forked {
                                session,
                            })
                        }
                        Ok(CommandOutcome::ModelUpdated { model }) => WireResponseBody::Command(
                            crate::protocol::CommandOutcome::ModelUpdated { model },
                        ),
                        Ok(CommandOutcome::CollaborationModeUpdated { mode }) => {
                            WireResponseBody::Command(
                                crate::protocol::CommandOutcome::CollaborationModeUpdated { mode },
                            )
                        }
                        Ok(CommandOutcome::AgentProfileUpdated { profile }) => {
                            WireResponseBody::Command(
                                crate::protocol::CommandOutcome::AgentProfileUpdated { profile },
                            )
                        }
                        Ok(CommandOutcome::Turn(turn)) => {
                            WireResponseBody::Command(crate::protocol::CommandOutcome::Submitted {
                                turn,
                            })
                        }
                        Ok(CommandOutcome::WorkspaceRestored { outcome }) => {
                            WireResponseBody::Command(
                                crate::protocol::CommandOutcome::WorkspaceRestored { outcome },
                            )
                        }
                        Ok(CommandOutcome::TransactionReviewed { outcome }) => {
                            WireResponseBody::Command(
                                crate::protocol::CommandOutcome::TransactionReviewed { outcome },
                            )
                        }
                        Ok(CommandOutcome::ExtensionResult { result }) => {
                            WireResponseBody::Command(
                                crate::protocol::CommandOutcome::ExtensionResult { result },
                            )
                        }
                        Ok(CommandOutcome::Ok) => {
                            if approval {
                                WireResponseBody::Command(
                                    crate::protocol::CommandOutcome::ApprovalRecorded,
                                )
                            } else {
                                WireResponseBody::Command(
                                    crate::protocol::CommandOutcome::Cancelled,
                                )
                            }
                        }
                        Err(error) => WireResponseBody::Error(error),
                    }
                }
                WireRequestBody::Query(query) => match query {
                    Query::SessionSnapshot(snapshot) => self
                        .application
                        .query_session(snapshot.session, snapshot.after_cursor)
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::SessionList => self
                        .application
                        .query_sessions()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::SkillCatalog(_) => self
                        .application
                        .query_skill_catalog()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::ModelCatalog(_) => self
                        .application
                        .query_model_catalog()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::AgentCatalog(_) => self
                        .application
                        .query_agent_catalog()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::AgentProfileCatalog(_) => self
                        .application
                        .query_agent_profile_catalog()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::MemoryStatus(_) => self
                        .application
                        .query_memory_status()
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::MemorySearch(request) => self
                        .application
                        .query_memory_search(request.query, request.session)
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::MemorySnapshot(request) => self
                        .application
                        .query_memory_snapshot(request.session, request.memory_type, request.limit)
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::CheckpointList(request) => self
                        .application
                        .query_checkpoint_list(request.session, request.limit)
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                    Query::TransactionSettlement(request) => self
                        .application
                        .query_transaction_settlement(request.session, request.transaction)
                        .await
                        .map(WireResponseBody::Query)
                        .unwrap_or_else(WireResponseBody::Error),
                },
            }
        };
        WireResponse {
            version: PROTOCOL_VERSION,
            request_id,
            body,
        }
    }
}

/// Typed command outcome returned to the Gateway client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Session(SessionRef),
    Updated(SessionRef),
    Forked(SessionRef),
    ModelUpdated { model: String },
    CollaborationModeUpdated { mode: RequestedCollaborationMode },
    AgentProfileUpdated { profile: String },
    Turn(TurnRef),
    WorkspaceRestored { outcome: WorkspaceRestoreOutcome },
    TransactionReviewed { outcome: serde_json::Value },
    ExtensionResult { result: serde_json::Value },
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
                workspace: body
                    .get("workspace")
                    .or_else(|| body.get("working_dir"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
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
                workspace: body
                    .get("workspace")
                    .or_else(|| body.get("working_dir"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                requested_target: Default::default(),
                requested_permission: Default::default(),
                required_agent_runtimes: Vec::new(),
                requested_task_kind: None,
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
                version: body.get("version").and_then(|v| v.as_u64()).unwrap_or(0),
                reason: None,
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
        async fn create_session(
            &self,
            hint: Option<String>,
            _workspace: Option<String>,
        ) -> Result<SessionRef, ProtocolError> {
            self.created.lock().unwrap().push(hint);
            Ok(SessionRef("sess-1".into()))
        }
        async fn resume_session(&self, reference: String) -> Result<SessionRef, ProtocolError> {
            Ok(SessionRef(reference))
        }
        async fn clear_session(&self, session: SessionRef) -> Result<SessionRef, ProtocolError> {
            Ok(session)
        }
        async fn fork_session(
            &self,
            _session: SessionRef,
            _through_sequence: u64,
        ) -> Result<SessionRef, ProtocolError> {
            Ok(SessionRef("child-1".into()))
        }
        async fn compact_session(&self, session: SessionRef) -> Result<SessionRef, ProtocolError> {
            Ok(session)
        }
        async fn submit_prompt(
            &self,
            _s: SessionRef,
            _c: String,
            _workspace: Option<String>,
            _target: RequestedExecutionTarget,
            _permission: RequestedPermissionMode,
            _required_agent_runtimes: Vec<String>,
            _requested_task_kind: Option<String>,
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
            _version: u64,
            _reason: Option<String>,
        ) -> Result<(), ProtocolError> {
            Ok(())
        }

        async fn query_skill_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({
                "skills": [{
                    "id": "demo:inspect",
                    "name": "inspect",
                    "description": "Inspect a workspace",
                    "enabled": true,
                    "extension_id": "demo"
                }]
            }))
        }

        async fn query_agent_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({
                "agents": [{"id": "agent-1", "status": "running"}]
            }))
        }

        async fn set_model(&self, model: String) -> Result<String, ProtocolError> {
            Ok(model)
        }

        async fn set_collaboration_mode(
            &self,
            mode: RequestedCollaborationMode,
        ) -> Result<RequestedCollaborationMode, ProtocolError> {
            Ok(mode)
        }

        async fn set_agent_profile(&self, profile: String) -> Result<String, ProtocolError> {
            Ok(profile)
        }

        async fn invoke_skill(
            &self,
            _request: crate::protocol::SkillInvokeRequest,
        ) -> Result<TurnRef, ProtocolError> {
            Ok(TurnRef("skill-turn-1".into()))
        }

        async fn restore_workspace_checkpoint(
            &self,
            _request: crate::protocol::RestoreWorkspaceCheckpoint,
        ) -> Result<crate::protocol::WorkspaceRestoreOutcome, ProtocolError> {
            Ok(crate::protocol::WorkspaceRestoreOutcome::Completed)
        }

        async fn execute_shell(
            &self,
            _request: crate::protocol::ExecuteShellRequest,
        ) -> Result<TurnRef, ProtocolError> {
            Ok(TurnRef("shell-turn-1".into()))
        }

        async fn review_transaction(
            &self,
            _request: crate::protocol::ReviewTransactionRequest,
        ) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"status": "accepted"}))
        }

        async fn query_agent_profile_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({
                "profiles": [{"name": "default"}]
            }))
        }

        async fn query_memory_status(&self) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"memory": {"local": "healthy"}}))
        }

        async fn query_memory_search(
            &self,
            query: String,
            _session: Option<String>,
        ) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"items": [{"content": query}]}))
        }

        async fn query_memory_snapshot(
            &self,
            _session: SessionRef,
            _memory_type: String,
            _limit: u16,
        ) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"content": "# Memory"}))
        }

        async fn query_checkpoint_list(
            &self,
            session: SessionRef,
            _limit: u16,
        ) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({
                "schema_version": ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
                "session_id": session.0,
                "checkpoints": [],
            }))
        }

        async fn query_transaction_settlement(
            &self,
            _session: SessionRef,
            _transaction: String,
        ) -> Result<serde_json::Value, ProtocolError> {
            Ok(serde_json::json!({"receipt": null}))
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
                workspace: None,
            }))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            CommandOutcome::Session(SessionRef("sess-1".into()))
        );
    }

    #[tokio::test]
    async fn typed_handler_routes_session_lifecycle_commands() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app);
        assert_eq!(
            handler
                .dispatch(Command::ClearSession(SessionRef("s-1".into())))
                .await
                .unwrap(),
            CommandOutcome::Updated(SessionRef("s-1".into()))
        );
        assert_eq!(
            handler
                .dispatch(Command::ForkSession(ForkSessionRequest {
                    session: SessionRef("s-1".into()),
                    through_sequence: 4,
                }))
                .await
                .unwrap(),
            CommandOutcome::Forked(SessionRef("child-1".into()))
        );
        assert_eq!(
            handler
                .dispatch(Command::CompactSession(SessionRef("s-1".into())))
                .await
                .unwrap(),
            CommandOutcome::Updated(SessionRef("s-1".into()))
        );
    }

    #[tokio::test]
    async fn typed_handler_routes_skill_catalog_query() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app);
        let response = handler
            .dispatch_wire(WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "catalog-1".into(),
                body: WireRequestBody::Query(Query::SkillCatalog(
                    crate::protocol::SkillCatalogQuery,
                )),
            })
            .await;
        match response.body {
            WireResponseBody::Query(value) => {
                assert_eq!(value["skills"][0]["name"], "inspect");
            }
            other => panic!("unexpected skill catalog response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn typed_handler_routes_agent_catalog_query() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app);
        let response = handler
            .dispatch_wire(WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "agent-catalog-1".into(),
                body: WireRequestBody::Query(Query::AgentCatalog(
                    crate::protocol::AgentCatalogQuery,
                )),
            })
            .await;
        match response.body {
            WireResponseBody::Query(value) => {
                assert_eq!(value["agents"][0]["id"], "agent-1");
            }
            other => panic!("unexpected agent catalog response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn typed_handler_routes_admin_preference_commands() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app);
        assert_eq!(
            handler
                .dispatch(Command::SetModel(SetModelRequest {
                    model: "sonnet".into(),
                }))
                .await
                .unwrap(),
            CommandOutcome::ModelUpdated {
                model: "sonnet".into()
            }
        );
        assert_eq!(
            handler
                .dispatch(Command::SetAgentProfile(SetAgentProfileRequest {
                    profile: "safe".into(),
                }))
                .await
                .unwrap(),
            CommandOutcome::AgentProfileUpdated {
                profile: "safe".into()
            }
        );
    }

    #[tokio::test]
    async fn typed_handler_routes_skill_and_memory_families() {
        let app: Arc<dyn TypedApplicationPort> = Arc::new(FakeApp {
            created: Mutex::new(vec![]),
        });
        let handler = TypedRouteHandler::new(app);
        assert_eq!(
            handler
                .dispatch(Command::InvokeSkill(crate::protocol::SkillInvokeRequest {
                    skill_id: "demo:inspect".into(),
                    user_args: "src".into(),
                    session: None,
                    workspace: None,
                }))
                .await
                .unwrap(),
            CommandOutcome::Turn(TurnRef("skill-turn-1".into()))
        );
        assert_eq!(
            handler
                .dispatch(Command::RestoreWorkspaceCheckpoint(
                    crate::protocol::RestoreWorkspaceCheckpoint {
                        session: SessionRef("s-1".into()),
                        prompt_index: 4,
                    },
                ))
                .await
                .unwrap(),
            CommandOutcome::WorkspaceRestored {
                outcome: crate::protocol::WorkspaceRestoreOutcome::Completed,
            }
        );
        assert_eq!(
            handler
                .dispatch(Command::ExecuteShell(
                    crate::protocol::ExecuteShellRequest {
                        command: "printf ok".into(),
                        session: Some(SessionRef("s-1".into())),
                        workspace: Some("/tmp".into()),
                        requested_permission: RequestedPermissionMode::Inherit,
                    },
                ))
                .await
                .unwrap(),
            CommandOutcome::Turn(TurnRef("shell-turn-1".into()))
        );
        assert_eq!(
            handler
                .dispatch(Command::ReviewTransaction(
                    crate::protocol::ReviewTransactionRequest {
                        session: SessionRef("s-1".into()),
                        transaction: "tx-1".into(),
                        action: ::contracts::TransactionReviewAction::Repair,
                        acknowledge_risk: false,
                    },
                ))
                .await
                .unwrap(),
            CommandOutcome::TransactionReviewed {
                outcome: serde_json::json!({"status": "accepted"}),
            }
        );
        let response = handler
            .dispatch_wire(WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "memory-1".into(),
                body: WireRequestBody::Query(Query::MemorySearch(
                    crate::protocol::MemorySearchQuery {
                        query: "needle".into(),
                        session: Some("s-1".into()),
                    },
                )),
            })
            .await;
        match response.body {
            WireResponseBody::Query(value) => assert_eq!(value["items"][0]["content"], "needle"),
            other => panic!("unexpected memory response: {other:?}"),
        }
        let response = handler
            .dispatch_wire(WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "memory-snapshot-1".into(),
                body: WireRequestBody::Query(Query::MemorySnapshot(
                    crate::protocol::MemorySnapshotQuery {
                        session: SessionRef("s-1".into()),
                        memory_type: "all".into(),
                        limit: 20,
                    },
                )),
            })
            .await;
        match response.body {
            WireResponseBody::Query(value) => assert_eq!(value["content"], "# Memory"),
            other => panic!("unexpected memory snapshot response: {other:?}"),
        }
        let response = handler
            .dispatch_wire(WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "checkpoint-list-1".into(),
                body: WireRequestBody::Query(Query::CheckpointList(
                    crate::protocol::CheckpointListQuery {
                        session: SessionRef("s-1".into()),
                        limit: 64,
                    },
                )),
            })
            .await;
        match response.body {
            WireResponseBody::Query(value) => {
                assert_eq!(value["session_id"], "s-1");
            }
            other => panic!("unexpected checkpoint response: {other:?}"),
        }
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
