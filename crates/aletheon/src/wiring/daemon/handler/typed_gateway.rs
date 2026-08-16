//! Official-socket adapter for the typed Gateway protocol.
//!
//! This is intentionally an application-boundary adapter: it resolves the
//! authenticated connection and delegates to existing use-case ports.  It
//! does not own a repository, mint an ID, or infer effective policy.

use async_trait::async_trait;
use gateway::protocol::{
    Cursor, ExecuteShellRequest, ExtensionRequest, ProtocolError, RequestedCollaborationMode,
    RequestedExecutionTarget, RequestedPermissionMode, RestoreWorkspaceCheckpoint,
    ReviewTransactionRequest, SessionRef, SkillInvokeRequest, TurnRef, WorkspaceRestoreOutcome,
};
use gateway::server::handlers::typed::TypedApplicationPort;

use super::{ports::HandlerPorts, RequestHandler};
use crate::wiring::approval_service::{ApprovalContext, ResolveApprovalRequest};
use crate::wiring::daemon::server::ConnectionContext;
use adapters_sqlite::approval_repository::ApprovalDecision;
use std::path::Path;
use std::sync::Arc;

/// Narrow host use cases required by the typed Gateway adapter. The adapter
/// receives this port instead of carrying the full RequestHandler/component
/// graph; legacy JSON-RPC remains the only caller that needs RequestHandler.
#[async_trait]
pub(crate) trait TypedDaemonUseCases: Send + Sync {
    async fn select_workspace_session(
        &self,
        working_dir: &Path,
    ) -> anyhow::Result<::contracts::ThreadId>;
    async fn submit_explicit_chat(
        &self,
        connection: &ConnectionContext,
        id: serde_json::Value,
        message: String,
        thread_id: ::contracts::ThreadId,
        workspace: ::contracts::WorkspacePolicy,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        permission_mode: ::contracts::permission::HostPermissionMode,
    ) -> anyhow::Result<runtime::TurnId>;
    async fn protocol_read_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot>;
}

#[async_trait]
impl TypedDaemonUseCases for RequestHandler {
    async fn select_workspace_session(
        &self,
        working_dir: &Path,
    ) -> anyhow::Result<::contracts::ThreadId> {
        RequestHandler::select_workspace_session(self, working_dir).await
    }

    async fn submit_explicit_chat(
        &self,
        connection: &ConnectionContext,
        id: serde_json::Value,
        message: String,
        thread_id: ::contracts::ThreadId,
        workspace: ::contracts::WorkspacePolicy,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        permission_mode: ::contracts::permission::HostPermissionMode,
    ) -> anyhow::Result<runtime::TurnId> {
        RequestHandler::submit_explicit_chat(
            self,
            connection,
            id,
            message,
            thread_id,
            workspace,
            requirements,
            task_kind,
            execution_target,
            permission_mode,
        )
        .await
    }

    async fn protocol_read_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot> {
        RequestHandler::protocol_read_snapshot_for(self, principal, session_id).await
    }
}

#[derive(Clone)]
pub(crate) struct DaemonTypedApplication {
    ports: Arc<HandlerPorts>,
    application: Arc<dyn TypedDaemonUseCases>,
    thread_authority: Arc<application::thread_authority::ThreadAuthorityStore>,
    grok_hardening: crate::config::GrokHardeningConfig,
    connection: ConnectionContext,
}

impl DaemonTypedApplication {
    pub(crate) fn from_parts(
        ports: Arc<HandlerPorts>,
        application: Arc<dyn TypedDaemonUseCases>,
        thread_authority: Arc<application::thread_authority::ThreadAuthorityStore>,
        grok_hardening: crate::config::GrokHardeningConfig,
        connection: ConnectionContext,
    ) -> Self {
        Self {
            ports,
            application,
            thread_authority,
            grok_hardening,
            connection,
        }
    }

    fn map_error(error: impl std::fmt::Display) -> ProtocolError {
        ProtocolError::Server(error.to_string())
    }

    /// The legacy turn coordinator uses the request id as its idempotency
    /// key. Typed Gateway commands are transport-correlated independently,
    /// so every submission must carry a fresh host correlation value; a
    /// constant such as `"gateway"` would replay the first turn forever for
    /// every later prompt on the same daemon.
    fn turn_request_id(prefix: &str) -> serde_json::Value {
        serde_json::json!(format!("{prefix}:{}", uuid::Uuid::new_v4()))
    }
}

#[async_trait]
impl TypedApplicationPort for DaemonTypedApplication {
    async fn create_session(
        &self,
        principal_hint: Option<String>,
        workspace: Option<String>,
    ) -> Result<SessionRef, ProtocolError> {
        // The hint is a requested preference.  The authenticated peer remains
        // authoritative and the legacy port applies that identity binding.
        if principal_hint.is_some() {
            tracing::debug!("typed Gateway ignored untrusted principal hint");
        }
        let session = if let Some(workspace) = workspace {
            let requested = std::path::PathBuf::from(workspace);
            let process_cwd = std::env::current_dir().map_err(Self::map_error)?;
            let effective = ::contracts::WorkspaceSelection::new(Some(requested), Vec::new())
                .resolve_with_profile(
                    &process_cwd,
                    &::contracts::PermissionProfileId::workspace_write(),
                )
                .map_err(Self::map_error)?;
            self.ports
                .sessions
                .route_workspace(effective.cwd().to_path_buf())
                .await
                .map_err(Self::map_error)?
        } else {
            self.ports
                .sessions
                .create()
                .await
                .map_err(Self::map_error)?
                .session_id
        };
        Ok(SessionRef(session))
    }

    async fn resume_session(&self, reference: String) -> Result<SessionRef, ProtocolError> {
        self.ports
            .sessions
            .resume(reference)
            .await
            .map(|snapshot| SessionRef(snapshot.session_id))
            .map_err(Self::map_error)
    }

    async fn clear_session(&self, session: SessionRef) -> Result<SessionRef, ProtocolError> {
        let transition = self
            .ports
            .sessions
            .clear(&session.0)
            .await
            .map_err(Self::map_error)?;
        self.ports
            .session_lifecycle
            .finish(
                transition.previous.session_id.clone(),
                transition.previous.turn_count,
            )
            .await
            .map_err(Self::map_error)?;
        self.ports.session_lifecycle.reset_turn_token().await;
        Ok(SessionRef(transition.current.session_id))
    }

    async fn fork_session(
        &self,
        session: SessionRef,
        through_sequence: u64,
    ) -> Result<SessionRef, ProtocolError> {
        let parent_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(session.0.clone()),
        );
        let settings = self
            .thread_authority
            .get(&parent_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| {
                ProtocolError::Server("no host-bound authority for parent session".into())
            })?;
        let record = self
            .ports
            .turn
            .session_fork(::contracts::SessionId(session.0), through_sequence)
            .await
            .map_err(Self::map_error)?;
        let child_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(record.id.0.clone()),
        );
        self.thread_authority
            .bind_or_verify(&child_key, &settings)
            .map_err(Self::map_error)?;
        Ok(SessionRef(record.id.0))
    }

    async fn compact_session(&self, session: SessionRef) -> Result<SessionRef, ProtocolError> {
        let transition = self
            .ports
            .sessions
            .compact(&session.0)
            .await
            .map_err(Self::map_error)?;
        Ok(SessionRef(
            transition
                .map(|value| value.current.session_id)
                .unwrap_or(session.0),
        ))
    }

    async fn set_model(&self, model: String) -> Result<String, ProtocolError> {
        self.ports
            .admin
            .switch_model(model)
            .await
            .map_err(Self::map_error)
    }

    async fn set_collaboration_mode(
        &self,
        mode: RequestedCollaborationMode,
    ) -> Result<RequestedCollaborationMode, ProtocolError> {
        let mode = match mode {
            RequestedCollaborationMode::Default => {
                application::turn_control::CollaborationMode::Default
            }
            RequestedCollaborationMode::Plan => application::turn_control::CollaborationMode::Plan,
            RequestedCollaborationMode::Auto => application::turn_control::CollaborationMode::Auto,
            RequestedCollaborationMode::Sandbox => {
                application::turn_control::CollaborationMode::Sandbox
            }
        };
        self.ports
            .admin
            .switch_mode(mode)
            .await
            .map_err(Self::map_error)?;
        Ok(match mode {
            application::turn_control::CollaborationMode::Default => {
                RequestedCollaborationMode::Default
            }
            application::turn_control::CollaborationMode::Plan => RequestedCollaborationMode::Plan,
            application::turn_control::CollaborationMode::Auto => RequestedCollaborationMode::Auto,
            application::turn_control::CollaborationMode::Sandbox => {
                RequestedCollaborationMode::Sandbox
            }
        })
    }

    async fn set_agent_profile(&self, profile: String) -> Result<String, ProtocolError> {
        self.ports
            .admin
            .switch_agent_profile(profile)
            .await
            .map(|result| result.current)
            .map_err(Self::map_error)
    }

    async fn invoke_skill(&self, request: SkillInvokeRequest) -> Result<TurnRef, ProtocolError> {
        let skill_id = request.skill_id.trim();
        if skill_id.is_empty() {
            return Err(ProtocolError::Server("skill_id is required".into()));
        }
        let descriptor = self
            .ports
            .admin
            .list_skills()
            .await
            .into_iter()
            .find(|skill| skill.enabled && skill.id == skill_id)
            .ok_or_else(|| {
                ProtocolError::Server(format!("Skill is unavailable or disabled: {skill_id}"))
            })?;
        let workspace_path = request
            .workspace
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_dir().map_err(Self::map_error)?);
        let process_cwd = std::env::current_dir().map_err(Self::map_error)?;
        let workspace = ::contracts::WorkspaceSelection::new(Some(workspace_path), Vec::new())
            .resolve_with_profile(
                &process_cwd,
                &::contracts::PermissionProfileId::workspace_write(),
            )
            .map_err(Self::map_error)?;
        let session = match request.session {
            Some(session) => ::contracts::ThreadId(session.0),
            None => self
                .application
                .select_workspace_session(workspace.cwd())
                .await
                .map_err(Self::map_error)?,
        };
        let user_args = request.user_args.trim();
        let message = if user_args.is_empty() {
            format!("Invoke the registered Skill `{}` now.", descriptor.name)
        } else {
            format!(
                "Invoke the registered Skill `{}` now.\n\nUser input:\n{}",
                descriptor.name, user_args
            )
        };
        let turn = self
            .application
            .submit_explicit_chat(
                &self.connection,
                Self::turn_request_id("typed-skill"),
                message,
                session,
                workspace,
                Vec::new(),
                None,
                ::contracts::ExecutionTargetSelection::default(),
                ::contracts::permission::HostPermissionMode::Safe,
            )
            .await
            .map_err(Self::map_error)?;
        Ok(TurnRef(turn.0))
    }

    async fn execute_shell(&self, request: ExecuteShellRequest) -> Result<TurnRef, ProtocolError> {
        let command = request.command.trim();
        if command.is_empty() {
            return Err(ProtocolError::Server("shell command is required".into()));
        }
        if command.len() > 64 * 1024 {
            return Err(ProtocolError::Server("shell command is too large".into()));
        }
        let permission_mode = match request.requested_permission {
            gateway::protocol::RequestedPermissionMode::Full => {
                ::contracts::permission::HostPermissionMode::Full
            }
            gateway::protocol::RequestedPermissionMode::Safe
            | gateway::protocol::RequestedPermissionMode::Inherit => {
                ::contracts::permission::HostPermissionMode::Safe
            }
        };
        let workspace_path = request
            .workspace
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_dir().map_err(Self::map_error)?);
        let profile = if permission_mode.is_full() {
            ::contracts::PermissionProfileId::danger_full_access()
        } else {
            ::contracts::PermissionProfileId::workspace_write()
        };
        let workspace = ::contracts::WorkspaceSelection::new(Some(workspace_path), Vec::new())
            .resolve_with_profile(&std::env::current_dir().map_err(Self::map_error)?, &profile)
            .map_err(Self::map_error)?;
        let session = match request.session {
            Some(session) => ::contracts::ThreadId(session.0),
            None => self
                .application
                .select_workspace_session(workspace.cwd())
                .await
                .map_err(Self::map_error)?,
        };
        let content = format!(
            "Execute the following user-requested shell command exactly through the `exec_command` capability and report its terminal result:\n{}",
            command
        );
        let turn = self
            .application
            .submit_explicit_chat(
                &self.connection,
                Self::turn_request_id("typed-shell"),
                content,
                session,
                workspace,
                vec![::contracts::TurnRequirement::InvokeCapability {
                    name: "exec_command".into(),
                }],
                None,
                ::contracts::ExecutionTargetSelection::default(),
                permission_mode,
            )
            .await
            .map_err(Self::map_error)?;
        Ok(TurnRef(turn.0))
    }

    async fn review_transaction(
        &self,
        request: ReviewTransactionRequest,
    ) -> Result<serde_json::Value, ProtocolError> {
        if request.session.0.trim().is_empty() || request.transaction.trim().is_empty() {
            return Err(ProtocolError::Server(
                "session and transaction are required".into(),
            ));
        }
        let transaction_id = uuid::Uuid::parse_str(&request.transaction)
            .map(::contracts::change_transaction::ChangeTransactionId)
            .map_err(|error| ProtocolError::Server(format!("invalid transaction_id: {error}")))?;
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(request.session.0.clone()),
        );
        let settings = self
            .thread_authority
            .get(&authority_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| ProtocolError::Server("session workspace authority not found".into()))?;
        let snapshot = self
            .application
            .protocol_read_snapshot_for(
                &self.connection.principal_id,
                &::contracts::SessionId(request.session.0.clone()),
            )
            .await
            .map_err(Self::map_error)?;
        let findings = snapshot
            .tasks
            .into_iter()
            .flat_map(|task| task.review_findings)
            .collect::<Vec<_>>();
        let action = match request.action {
            ::contracts::TransactionReviewAction::Accept => {
                application::settlement::TransactionReviewAction::Accept
            }
            ::contracts::TransactionReviewAction::Repair => {
                application::settlement::TransactionReviewAction::Repair
            }
            ::contracts::TransactionReviewAction::Rollback => {
                application::settlement::TransactionReviewAction::Rollback
            }
        };
        let outcome = self
            .ports
            .transaction_review
            .review(
                action,
                transaction_id,
                &request.session.0,
                settings.workspace.cwd(),
                &findings,
                request.acknowledge_risk,
            )
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(outcome).map_err(Self::map_error)
    }

    async fn manage_extension(
        &self,
        request: ExtensionRequest,
    ) -> Result<serde_json::Value, ProtocolError> {
        let actor = self.connection.principal_id.0.as_str();
        let value = match request {
            ExtensionRequest::Install {
                path,
                trust_workspace,
                approve_permissions: _,
            } => {
                let path = std::fs::canonicalize(path).map_err(Self::map_error)?;
                serde_json::to_value(
                    self.ports
                        .extensions
                        .install(actor, &path, trust_workspace)
                        .await
                        .map_err(Self::map_error)?,
                )
                .map_err(Self::map_error)?
            }
            ExtensionRequest::List => {
                serde_json::to_value(self.ports.extensions.list().map_err(Self::map_error)?)
                    .map_err(Self::map_error)?
            }
            ExtensionRequest::Show { id } => {
                serde_json::to_value(self.ports.extensions.show(&id).map_err(Self::map_error)?)
                    .map_err(Self::map_error)?
            }
            ExtensionRequest::Enable {
                id,
                approve_permissions,
            } => serde_json::to_value(
                self.ports
                    .extensions
                    .enable(actor, &id, approve_permissions)
                    .await
                    .map_err(Self::map_error)?,
            )
            .map_err(Self::map_error)?,
            ExtensionRequest::Disable { id } => serde_json::to_value(
                self.ports
                    .extensions
                    .disable(actor, &id)
                    .await
                    .map_err(Self::map_error)?,
            )
            .map_err(Self::map_error)?,
            ExtensionRequest::Upgrade {
                path,
                trust_workspace,
                approve_permissions,
            } => {
                let path = std::fs::canonicalize(path).map_err(Self::map_error)?;
                serde_json::to_value(
                    self.ports
                        .extensions
                        .upgrade(actor, &path, trust_workspace, approve_permissions)
                        .await
                        .map_err(Self::map_error)?,
                )
                .map_err(Self::map_error)?
            }
            ExtensionRequest::Rollback { id } => serde_json::to_value(
                self.ports
                    .extensions
                    .rollback(actor, &id)
                    .await
                    .map_err(Self::map_error)?,
            )
            .map_err(Self::map_error)?,
            ExtensionRequest::Remove { id } => serde_json::to_value(
                self.ports
                    .extensions
                    .remove(actor, &id)
                    .await
                    .map_err(Self::map_error)?,
            )
            .map_err(Self::map_error)?,
            ExtensionRequest::Purge { id } => serde_json::to_value(
                self.ports
                    .extensions
                    .purge(actor, &id)
                    .await
                    .map_err(Self::map_error)?,
            )
            .map_err(Self::map_error)?,
            ExtensionRequest::Doctor { id } => {
                serde_json::to_value(self.ports.extensions.doctor(&id).map_err(Self::map_error)?)
                    .map_err(Self::map_error)?
            }
        };
        Ok(value)
    }

    async fn restore_workspace_checkpoint(
        &self,
        request: RestoreWorkspaceCheckpoint,
    ) -> Result<WorkspaceRestoreOutcome, ProtocolError> {
        if !self.grok_hardening.workspace_checkpoint {
            return Err(ProtocolError::Server(
                "workspace checkpoint feature is disabled".into(),
            ));
        }
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(request.session.0.clone()),
        );
        let settings = self
            .thread_authority
            .get(&authority_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| ProtocolError::Server("no host-bound authority for session".into()))?;
        let outcome = self
            .ports
            .turn
            .rewind_workspace(
                self.connection.principal_id.clone(),
                request.session.0,
                request.prompt_index,
                application::workspace_checkpoint::WorkspaceIdentity {
                    canonical_path: settings.workspace.cwd().to_path_buf(),
                    repo_fingerprint: None,
                },
            )
            .await;
        Ok(match outcome {
            application::workspace_checkpoint::RestoreOutcome::Completed => {
                WorkspaceRestoreOutcome::Completed
            }
            application::workspace_checkpoint::RestoreOutcome::IdentityMismatch => {
                WorkspaceRestoreOutcome::IdentityMismatch
            }
            application::workspace_checkpoint::RestoreOutcome::UnprotectedChangesAbort => {
                WorkspaceRestoreOutcome::UnprotectedChangesAbort
            }
            application::workspace_checkpoint::RestoreOutcome::FsRestoreFailed { detail } => {
                WorkspaceRestoreOutcome::FsRestoreFailed { detail }
            }
            application::workspace_checkpoint::RestoreOutcome::Partial { detail } => {
                WorkspaceRestoreOutcome::Partial { detail }
            }
        })
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
    ) -> Result<TurnRef, ProtocolError> {
        let execution_target = match requested_target {
            RequestedExecutionTarget::Automatic => ::contracts::ExecutionTargetSelection::default(),
            RequestedExecutionTarget::General => ::contracts::ExecutionTargetSelection::general(
                ::contracts::ExecutionTargetSource::TrustedClient,
            ),
            RequestedExecutionTarget::Robot {
                device_id,
                environment,
            } => {
                let environment = match environment.as_str() {
                    "simulation" => {
                        ::contracts::types::embodiment::ExecutionEnvironment::Simulation
                    }
                    "hil" => ::contracts::types::embodiment::ExecutionEnvironment::Hil,
                    "real" => ::contracts::types::embodiment::ExecutionEnvironment::Real,
                    other => {
                        return Err(ProtocolError::Server(format!(
                            "unsupported execution environment: {other}"
                        )))
                    }
                };
                ::contracts::ExecutionTargetSelection::robot(
                    device_id,
                    environment,
                    ::contracts::ExecutionTargetSource::TrustedClient,
                )
                .map_err(ProtocolError::Server)?
            }
            RequestedExecutionTarget::Runtime(runtime_id) => {
                tracing::debug!(
                    runtime_id,
                    "typed Gateway runtime preference deferred to host policy"
                );
                ::contracts::ExecutionTargetSelection::default()
            }
        };
        if !matches!(&requested_permission, RequestedPermissionMode::Inherit) {
            tracing::debug!(
                ?requested_permission,
                "typed Gateway permission preference deferred to host policy"
            );
        }
        let requirements = required_agent_runtimes
            .into_iter()
            .map(|runtime_id| ::contracts::TurnRequirement::InvokeAgentRuntime { runtime_id })
            .collect();
        let task_kind = match requested_task_kind.as_deref() {
            None => None,
            Some("coding") => Some(::contracts::TaskKind::Coding),
            Some(other) => {
                return Err(ProtocolError::Server(format!(
                    "unsupported requested task kind: {other}"
                )))
            }
        };
        let permission_mode = match requested_permission {
            RequestedPermissionMode::Full => ::contracts::permission::HostPermissionMode::Full,
            RequestedPermissionMode::Safe => ::contracts::permission::HostPermissionMode::Safe,
            RequestedPermissionMode::Inherit => ::contracts::permission::HostPermissionMode::Safe,
        };
        let session_id = session.0;
        let workspace_path = workspace
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_dir().map_err(Self::map_error)?);
        let profile = if permission_mode.is_full() {
            ::contracts::PermissionProfileId::danger_full_access()
        } else {
            ::contracts::PermissionProfileId::workspace_write()
        };
        let workspace = ::contracts::WorkspaceSelection::new(Some(workspace_path), Vec::new())
            .resolve_with_profile(&std::env::current_dir().map_err(Self::map_error)?, &profile)
            .map_err(Self::map_error)?;
        let runtime_turn = self
            .application
            .submit_explicit_chat(
                &self.connection,
                Self::turn_request_id("gateway"),
                content,
                ::contracts::ThreadId(session_id),
                workspace,
                requirements,
                task_kind,
                execution_target,
                permission_mode,
            )
            .await
            .map_err(Self::map_error)?;
        Ok(TurnRef(runtime_turn.0))
    }

    async fn cancel_turn(&self, _session: SessionRef) -> Result<(), ProtocolError> {
        let cancelled = self
            .ports
            .turn
            .cancel_active_for_connection(self.connection.connection_id.clone())
            .await;
        if cancelled == 0 {
            return Err(ProtocolError::Server(
                "no active turn for this connection".into(),
            ));
        }
        Ok(())
    }

    async fn submit_approval(
        &self,
        _session: SessionRef,
        choice: String,
        approved: bool,
        version: u64,
        reason: Option<String>,
    ) -> Result<(), ProtocolError> {
        // SocketApprovalGate approvals are transient, connection-owned
        // requests. Their opaque choice id is intentionally not the durable
        // ApprovalRepository UUID, even though both are currently UUID-shaped
        // strings. Resolve the transient authority first; only fall back to
        // the durable ApprovalUseCases route when no pending socket request
        // belongs to this authenticated connection.
        let pending = self
            .ports
            .pending_approvals
            .scope_subject_authenticated(
                &self.connection.principal_id,
                &self.connection.connection_id,
                &choice,
            )
            .await;
        match pending {
            Ok(_scope_subject) => {
                let decision = if approved {
                    ::contracts::protocol::client::TransientApprovalDecision::Approve
                } else {
                    ::contracts::protocol::client::TransientApprovalDecision::Deny
                };
                self.ports
                    .admin
                    .resolve_transient_approval(
                        crate::wiring::application::admin_service::TransientApprovalRequest {
                            principal_id: self.connection.principal_id.clone(),
                            connection_id: self.connection.connection_id.clone(),
                            approval_id: choice,
                            decision,
                            // The typed command exposes approve/deny only;
                            // scoped path approval remains an explicit legacy
                            // capability and therefore has no hint here.
                            scope_hint: None,
                        },
                    )
                    .await
                    .map(|_| ())
                    .map_err(Self::map_error)
            }
            Err(crate::wiring::application::admin_service::PendingApprovalError::WrongOwner) => {
                Err(ProtocolError::Server(
                    "approval is not owned by this authenticated connection".into(),
                ))
            }
            Err(crate::wiring::application::admin_service::PendingApprovalError::NotFound) => {
                let approval_id = uuid::Uuid::parse_str(&choice).map_err(|_| {
                    ProtocolError::Server("approval choice_id must be a UUID".into())
                })?;
                let decision = if approved {
                    ApprovalDecision::Approve
                } else {
                    ApprovalDecision::Reject { reason }
                };
                self.ports
                    .approvals
                    .resolve(ResolveApprovalRequest {
                        context: ApprovalContext {
                            principal_id: self.connection.principal_id.clone(),
                            channel: "typed_gateway".into(),
                        },
                        approval_id: ::contracts::ApprovalId(approval_id),
                        version,
                        decision,
                    })
                    .await
                    .map(|_| ())
                    .map_err(Self::map_error)
            }
        }
    }

    async fn query_session(
        &self,
        session: SessionRef,
        after: Option<Cursor>,
    ) -> Result<serde_json::Value, ProtocolError> {
        let after_cursor = after
            .map(|cursor| ::contracts::protocol::client::EventCursor {
                sequence: cursor.sequence,
                event_id: cursor.event_id,
            })
            .unwrap_or_else(::contracts::protocol::client::EventCursor::origin);
        let snapshot = self
            .ports
            .session_gateway
            .protocol_read_snapshot_for(
                &self.connection.principal_id,
                &::contracts::SessionId(session.0.clone()),
            )
            .await
            .map_err(Self::map_error)?;
        let page = self
            .ports
            .session_gateway
            .protocol_event_page_for(
                &self.connection.principal_id,
                &::contracts::SessionId(session.0.clone()),
                &after_cursor,
            )
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(serde_json::json!({
            "snapshot": snapshot,
            "events": page.events,
            "after": page.after,
            "next": page.next,
        }))
        .map_err(Self::map_error)
    }

    async fn query_sessions(&self) -> Result<serde_json::Value, ProtocolError> {
        let list = self
            .ports
            .session_gateway
            .protocol_session_list_for(&self.connection.principal_id)
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(list).map_err(Self::map_error)
    }

    async fn query_skill_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        let skills = self.ports.admin.list_skills().await;
        let skills = skills
            .into_iter()
            .map(|skill| {
                serde_json::json!({
                    "id": skill.id,
                    "name": skill.name,
                    "description": skill.description,
                    "enabled": skill.enabled,
                    "extension_id": skill.extension_id,
                })
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({"skills": skills}))
    }

    async fn query_model_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        self.ports
            .admin
            .model_catalog()
            .await
            .map(|catalog| {
                serde_json::json!({
                    "models": catalog.models,
                    "current": catalog.current,
                })
            })
            .map_err(Self::map_error)
    }

    async fn query_agent_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        let agents = self
            .ports
            .admin
            .sub_agents()
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(serde_json::json!({"agents": agents})).map_err(Self::map_error)
    }

    async fn query_agent_profile_catalog(&self) -> Result<serde_json::Value, ProtocolError> {
        let profiles = self
            .ports
            .admin
            .list_agent_profiles()
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(serde_json::json!({"profiles": profiles})).map_err(Self::map_error)
    }

    async fn query_memory_status(&self) -> Result<serde_json::Value, ProtocolError> {
        let health = self.ports.memory_health_snapshot();
        Ok(serde_json::json!({
            "memory": {
                "provider": "composite",
                "local": "healthy",
                "supplemental": {
                    "enabled": health.supplemental_enabled,
                    "state": if health.degraded { "degraded" } else { "healthy" },
                    "error_category": health.error_category.map(|value| format!("{value:?}").to_ascii_lowercase()),
                    "queue_depth": health.queue_depth,
                }
            }
        }))
    }

    async fn query_memory_search(
        &self,
        query: String,
        session: Option<String>,
    ) -> Result<serde_json::Value, ProtocolError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ProtocolError::Server("memory query is required".into()));
        }
        let working_dir = std::env::current_dir().map_err(Self::map_error)?;
        let client_session_id =
            session.unwrap_or_else(|| format!("typed-gateway-{}", self.connection.connection_id.0));
        let recall = self
            .ports
            .memory_gateway
            .recall(
                &self.connection.principal_id,
                "typed_gateway",
                ::contracts::protocol::memory::MemoryRecallRequestV1 {
                    request_id: format!("memory-{}", self.connection.connection_id.0),
                    client_session_id,
                    working_dir,
                    query: query.to_owned(),
                    max_items: 20,
                    max_content_bytes: 64 * 1024,
                    include_historical: false,
                    requested_kinds: None,
                },
            )
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(recall).map_err(|error| {
            ProtocolError::Server(format!("memory result encoding failed: {error}"))
        })
    }

    async fn query_memory_snapshot(
        &self,
        session: SessionRef,
        memory_type: String,
        limit: u16,
    ) -> Result<serde_json::Value, ProtocolError> {
        let memory_type = memory_type.trim();
        if !matches!(memory_type, "all" | "core" | "recall") {
            return Err(ProtocolError::Server(
                "memory_type must be all, core, or recall".into(),
            ));
        }
        let limit = usize::from(limit);
        if !(1..=256).contains(&limit) {
            return Err(ProtocolError::Server(
                "memory snapshot limit must be between 1 and 256".into(),
            ));
        }
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(session.0.clone()),
        );
        self.thread_authority
            .get(&authority_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| ProtocolError::Server("no host-bound authority for session".into()))?;
        self.ports
            .session_memory
            .snapshot(memory_type, limit)
            .await
            .map_err(Self::map_error)
    }

    async fn query_transaction_settlement(
        &self,
        session: SessionRef,
        transaction: String,
    ) -> Result<serde_json::Value, ProtocolError> {
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(session.0.clone()),
        );
        self.thread_authority
            .get(&authority_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| ProtocolError::Server("session workspace authority not found".into()))?;
        let receipt = self
            .ports
            .transaction_review
            .latest(&session.0, &transaction)
            .await
            .map_err(Self::map_error)?;
        serde_json::to_value(serde_json::json!({"receipt": receipt})).map_err(Self::map_error)
    }

    async fn query_checkpoint_list(
        &self,
        session: SessionRef,
        limit: u16,
    ) -> Result<serde_json::Value, ProtocolError> {
        if !self.grok_hardening.workspace_checkpoint {
            return Err(ProtocolError::Server(
                "workspace checkpoint feature is disabled".into(),
            ));
        }
        let limit = usize::from(limit);
        if !(1..=256).contains(&limit) {
            return Err(ProtocolError::Server(
                "checkpoint list limit must be between 1 and 256".into(),
            ));
        }
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            self.connection.principal_id.clone(),
            ::contracts::ThreadId(session.0.clone()),
        );
        self.thread_authority
            .get(&authority_key)
            .map_err(Self::map_error)?
            .ok_or_else(|| ProtocolError::Server("no host-bound authority for session".into()))?;
        let checkpoints = self
            .ports
            .workspace_checkpoint
            .list_session_checkpoints(&session.0, limit)
            .await
            .map_err(Self::map_error)?;
        let mut entries = Vec::with_capacity(checkpoints.len());
        for checkpoint in checkpoints {
            let turn_id = uuid::Uuid::parse_str(&checkpoint.turn_id).map_err(|error| {
                ProtocolError::Server(format!(
                    "checkpoint list contains invalid turn identity: {error}"
                ))
            })?;
            let through_sequence = self
                .ports
                .turn
                .session_sequence_through_turn(
                    ::contracts::SessionId(session.0.clone()),
                    ::contracts::TurnId(turn_id),
                )
                .await
                .map_err(Self::map_error)?;
            entries.push(serde_json::json!({
                "checkpoint_id": checkpoint.checkpoint_id.0.to_string(),
                "turn_id": checkpoint.turn_id,
                "prompt_index": checkpoint.prompt_index,
                "through_sequence": through_sequence,
                "created_at_ms": checkpoint.created_at_ms,
                "finalized": matches!(
                    checkpoint.finalize_state,
                    application::workspace_checkpoint::CheckpointFinalizeState::Finalized
                ),
            }));
        }
        Ok(serde_json::json!({
            "schema_version": ::contracts::CHECKPOINT_LIST_SCHEMA_VERSION,
            "session_id": session.0,
            "checkpoints": entries,
        }))
    }
}
