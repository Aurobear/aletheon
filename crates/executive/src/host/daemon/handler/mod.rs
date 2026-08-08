//! Daemon request handler — JSON-RPC dispatcher for the Unix socket server.
//! Handles chat, RPC, session management, and lifecycle events.

mod connection;
pub(crate) mod format;
mod init;
pub(crate) mod ports;
mod rpc;
pub(crate) mod tool_executor;
mod turn_handler;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::application::{CommandDispatcher, CommandOutput, CommandUseCases};
use crate::composition::config::GrokHardeningConfig;

#[derive(Clone)]
pub struct RequestHandler {
    /// Narrow application use cases available to protocol handlers.
    pub(crate) ports: Arc<ports::HandlerPorts>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// Host-owned connection admission limit.
    pub(crate) max_connections: Option<usize>,
    /// User-state-root-scoped immutable thread authority records.
    pub(crate) thread_authority: Arc<crate::application::thread_authority::ThreadAuthorityStore>,
    /// Feature flags for Grok-hardening mechanisms (folder_trust, etc.).
    pub(crate) grok_hardening: GrokHardeningConfig,
    /// Principal-scoped gate for repository-provided executable configuration.
    pub(crate) workspace_trust: Arc<crate::application::workspace_trust::WorkspaceTrustResolver>,
    /// Retained optional MCP runtime for health projection and bounded shutdown.
    pub(crate) mcp: Option<Arc<corpus::tools::mcp::manager::McpManager>>,
}

struct DaemonCommandUseCases {
    handler: RequestHandler,
    connection: super::server::ConnectionContext,
    rpc_id: serde_json::Value,
}

#[async_trait::async_trait]
impl CommandUseCases for DaemonCommandUseCases {
    async fn submit_prompt(
        &self,
        intent: &fabric::contract::command::ClientIntent,
        prompt: &fabric::contract::command::SubmitPromptIntent,
    ) -> anyhow::Result<CommandOutput> {
        let thread_id = match &prompt.session_id {
            Some(session_id) => fabric::ThreadId(session_id.0.clone()),
            None => {
                self.handler
                    .select_workspace_session(prompt.workspace.cwd())
                    .await?
            }
        };
        let response = self
            .handler
            .execute_explicit_chat(
                &self.connection,
                self.rpc_id.clone(),
                prompt.content.clone(),
                thread_id,
                prompt.workspace.clone(),
                prompt.requirements.clone(),
                prompt.task_kind,
                prompt.execution_target.clone(),
                prompt.permission_mode,
            )
            .await;
        if let Some((code, message)) = rpc_error_parts(&response) {
            return Ok(CommandOutput::new(
                intent.correlation_id.clone(),
                fabric::contract::command::CommandOutputV1::Rejected(
                    fabric::contract::command::CommandRejectionV1 { code, message },
                ),
            ));
        }
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            fabric::contract::command::CommandOutputV1::PromptCompleted(
                crate::application::command_dispatcher::prompt_completion_from_rpc_result(
                    take_rpc_result(response)?,
                )?,
            ),
        ))
    }

    async fn execute_shell(
        &self,
        intent: &fabric::contract::command::ClientIntent,
        shell: &fabric::contract::command::ExecuteShellIntent,
    ) -> anyhow::Result<CommandOutput> {
        let thread_id = match &shell.session_id {
            Some(session_id) => fabric::ThreadId(session_id.0.clone()),
            None => {
                self.handler
                    .select_workspace_session(shell.workspace.cwd())
                    .await?
            }
        };
        // The shell sigil is an intent adapter, never a local execution path.
        // A host-authored capability obligation routes the command through the
        // ordinary policy, approval, audit, receipt, and Activity projection.
        let content = format!(
            "Execute the following user-requested shell command exactly through the `exec_command` capability and report its terminal result:\n{}",
            shell.command
        );
        let response = self
            .handler
            .execute_explicit_chat(
                &self.connection,
                self.rpc_id.clone(),
                content,
                thread_id,
                shell.workspace.clone(),
                vec![fabric::TurnRequirement::InvokeCapability {
                    name: "exec_command".into(),
                }],
                None,
                fabric::ExecutionTargetSelection::default(),
                shell.permission_mode,
            )
            .await;
        if let Some((code, message)) = rpc_error_parts(&response) {
            return Ok(CommandOutput::new(
                intent.correlation_id.clone(),
                fabric::contract::command::CommandOutputV1::Rejected(
                    fabric::contract::command::CommandRejectionV1 { code, message },
                ),
            ));
        }
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            fabric::contract::command::CommandOutputV1::PromptCompleted(
                crate::application::command_dispatcher::prompt_completion_from_rpc_result(
                    take_rpc_result(response)?,
                )?,
            ),
        ))
    }

    async fn status(
        &self,
        intent: &fabric::contract::command::ClientIntent,
        status: &fabric::contract::command::StatusIntent,
    ) -> anyhow::Result<CommandOutput> {
        let Some(session_id) = status.session_id.as_ref() else {
            return Ok(CommandOutput::new(
                intent.correlation_id.clone(),
                fabric::contract::command::CommandOutputV1::Rejected(
                    fabric::contract::command::CommandRejectionV1 {
                        code: -32602,
                        message: "Missing session_id parameter".into(),
                    },
                ),
            ));
        };
        match self.handler.status_projection(&session_id.0).await {
            Ok(projection) => Ok(CommandOutput::new(
                intent.correlation_id.clone(),
                fabric::contract::command::CommandOutputV1::StatusProjected(projection),
            )),
            Err(error) => Ok(CommandOutput::new(
                intent.correlation_id.clone(),
                fabric::contract::command::CommandOutputV1::Rejected(
                    fabric::contract::command::CommandRejectionV1 {
                        code: -32000,
                        message: error.to_string(),
                    },
                ),
            )),
        }
    }
}

impl RequestHandler {
    pub(crate) async fn memory_observe(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryObservationRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryObservationReceiptV1> {
        self.ports
            .memory_gateway
            .observe(&connection.principal_id, "versioned_local_rpc", request)
            .await
    }

    pub(crate) async fn memory_receipt(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryReceiptGetRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryLifecycleReceiptV1> {
        self.ports
            .memory_gateway
            .receipt(&connection.principal_id, request)
            .await
    }

    pub(crate) async fn memory_recall(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryRecallRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryRecallResultV1> {
        self.ports
            .memory_gateway
            .recall(&connection.principal_id, "versioned_local_rpc", request)
            .await
    }

    pub(crate) async fn memory_feedback(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryFeedbackRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryFeedbackReceiptV1> {
        self.ports
            .memory_gateway
            .feedback(&connection.principal_id, "versioned_local_rpc", request)
            .await
    }

    pub(crate) async fn memory_workspace_preview_bind(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryWorkspacePreviewBindRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryWorkspaceBindingPreviewV1> {
        self.ports
            .memory_gateway
            .preview_workspace_bind(&connection.principal_id, request)
            .await
    }

    pub(crate) async fn memory_workspace_bind(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryWorkspaceBindRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryWorkspaceBindingViewV1> {
        self.ports
            .memory_gateway
            .bind_workspace(&connection.principal_id, request)
            .await
    }

    pub(crate) async fn memory_workspace_unbind(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory::MemoryWorkspaceUnbindRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory::MemoryWorkspaceBindingViewV1> {
        self.ports
            .memory_gateway
            .unbind_workspace(&connection.principal_id, request)
            .await
    }

    pub(crate) async fn memory_maintenance_status(
        &self,
        request: fabric::protocol::memory_maintenance::MemoryMaintenanceStatusRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory_maintenance::MemoryMaintenanceStatusV1> {
        self.ports.memory_maintenance.status(request).await
    }

    pub(crate) async fn memory_maintenance_run(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::memory_maintenance::MemoryMaintenanceRunRequestV1,
    ) -> anyhow::Result<fabric::protocol::memory_maintenance::MemoryMaintenanceRunReceiptV1> {
        let owner = format!("memory-agent:{}", connection.connection_id.0);
        self.ports.memory_maintenance.run(&owner, request).await
    }

    pub(crate) async fn resolve_versioned_approval(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::client::ApprovalRequest,
    ) -> anyhow::Result<fabric::ApprovalSnapshot> {
        self.ports
            .turn
            .verify_active(
                connection.principal_id.clone(),
                request.thread_id.0,
                request.turn_id,
                request.operation_id,
            )
            .await?;
        let decision = match request.decision {
            fabric::protocol::client::ApprovalDecisionRequest::Approve => {
                crate::application::approval::ApprovalDecision::Approve
            }
            fabric::protocol::client::ApprovalDecisionRequest::Reject => {
                crate::application::approval::ApprovalDecision::Reject {
                    reason: request.reason,
                }
            }
        };
        self.ports
            .approvals
            .resolve(
                crate::application::approval_service::ResolveApprovalRequest {
                    context: crate::application::approval_service::ApprovalContext {
                        principal_id: connection.principal_id.clone(),
                        channel: "versioned_local_rpc".into(),
                    },
                    approval_id: request.approval_id,
                    version: request.version,
                    decision,
                },
            )
            .await
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn cancel_versioned_turn(
        &self,
        connection: &super::server::ConnectionContext,
        request: fabric::protocol::client::CancelRequest,
    ) -> anyhow::Result<()> {
        self.ports
            .turn
            .cancel_by_key(
                connection.principal_id.clone(),
                request.thread_id.0,
                request.turn_id,
                request.operation_id,
            )
            .await
    }

    pub(crate) async fn protocol_snapshot_for(
        &self,
        principal: &fabric::PrincipalId,
        session_id: &fabric::SessionId,
    ) -> anyhow::Result<fabric::protocol::client::UiSnapshot> {
        self.ports
            .session_gateway
            .protocol_snapshot_for(principal, session_id)
            .await
    }

    pub(crate) async fn protocol_read_snapshot(
        &self,
        session_id: &fabric::SessionId,
    ) -> anyhow::Result<fabric::protocol::client::SessionReadSnapshot> {
        self.ports
            .session_gateway
            .protocol_read_snapshot(session_id)
            .await
    }

    pub(crate) async fn protocol_read_snapshot_for(
        &self,
        principal: &fabric::PrincipalId,
        session_id: &fabric::SessionId,
    ) -> anyhow::Result<fabric::protocol::client::SessionReadSnapshot> {
        self.ports
            .session_gateway
            .protocol_read_snapshot_for(principal, session_id)
            .await
    }

    pub(crate) async fn protocol_session_list_for(
        &self,
        principal: &fabric::PrincipalId,
    ) -> anyhow::Result<fabric::protocol::client::SessionListSnapshot> {
        self.ports
            .session_gateway
            .protocol_session_list_for(principal)
            .await
    }

    pub(crate) async fn protocol_events_after_for(
        &self,
        principal: &fabric::PrincipalId,
        session_id: &fabric::SessionId,
        after: &fabric::protocol::client::EventCursor,
    ) -> anyhow::Result<Vec<fabric::protocol::client::ClientEvent>> {
        self.ports
            .session_gateway
            .protocol_events_after_for(principal, session_id, after)
            .await
    }

    pub(crate) async fn protocol_event_page_for(
        &self,
        principal: &fabric::PrincipalId,
        session_id: &fabric::SessionId,
        after: &fabric::protocol::client::EventCursor,
    ) -> anyhow::Result<fabric::protocol::client::SessionEventPage> {
        self.ports
            .session_gateway
            .protocol_event_page_for(principal, session_id, after)
            .await
    }

    pub(crate) async fn cleanup_disconnected_connection(
        &self,
        connection_id: &fabric::ConnectionId,
    ) -> anyhow::Result<Vec<fabric::ProcessId>> {
        self.ports
            .pending_approvals
            .cancel_connection(connection_id)
            .await;
        // Cancel exactly the turns admitted by this connection before the
        // connection-scoped request tasks are aborted. Each turn's settlement
        // guard still guarantees active-index removal and kernel terminal
        // settlement even if its request future is dropped below.
        self.ports
            .turn
            .cancel_active_for_connection(connection_id.clone())
            .await;
        self.ports
            .kernel
            .cleanup_disconnected_connection(connection_id)
            .await
    }
    async fn handle_workspace_trust_evaluate(
        &self,
        connection: &super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let workspace = match resolve_requested_workspace(&request["params"]) {
            Ok(workspace) => workspace,
            Err(error) => return rpc_error(id, -32602, error),
        };
        let decision = self
            .workspace_trust
            .evaluate(
                connection.principal_id.clone(),
                crate::application::workspace_trust::workspace_identity(workspace.cwd()),
                fabric::workspace_trust::ClientMode::Interactive,
                crate::application::workspace_trust::is_broad_unrecordable_root(workspace.cwd()),
                unix_now(),
            )
            .await;
        let result = match decision {
            fabric::workspace_trust::WorkspaceTrustDecision::Trusted { granted } => {
                serde_json::json!({"decision":"trusted","sources":granted})
            }
            fabric::workspace_trust::WorkspaceTrustDecision::Restricted { blocked } => {
                serde_json::json!({"decision":"restricted","sources":blocked})
            }
            fabric::workspace_trust::WorkspaceTrustDecision::PromptRequired { findings } => {
                serde_json::json!({"decision":"prompt_required","sources":findings})
            }
        };
        serde_json::json!({"jsonrpc":"2.0","id":id,"result":result})
    }

    async fn handle_workspace_trust_grant(
        &self,
        connection: &super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let workspace = match resolve_requested_workspace(&request["params"]) {
            Ok(workspace) => workspace,
            Err(error) => return rpc_error(id, -32602, error),
        };
        let granted = match serde_json::from_value::<
            Vec<fabric::workspace_trust::ExecutableConfigSource>,
        >(request["params"]["granted"].clone())
        {
            Ok(granted) => granted,
            Err(error) => {
                return rpc_error(id, -32602, format!("invalid granted sources: {error}"))
            }
        };
        match self
            .workspace_trust
            .grant_current(
                connection.principal_id.clone(),
                crate::application::workspace_trust::workspace_identity(workspace.cwd()),
                granted,
                connection.connection_id.0.to_string(),
                unix_now(),
            )
            .await
        {
            Ok(receipt) => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{
                "decision":"trusted", "granted":receipt.granted,
                "updated_at_unix":receipt.updated_at_unix
            }}),
            Err(error) => rpc_error(id, -32040, error),
        }
    }

    /// Complete daemon-owned subsystem shutdown after transports stop accepting work.
    pub async fn shutdown_runtime(&self) -> anyhow::Result<()> {
        if let Some(mcp) = &self.mcp {
            let report = mcp.shutdown(std::time::Duration::from_secs(5)).await;
            if !report.aborted_tasks.is_empty() {
                tracing::warn!(tasks = ?report.aborted_tasks, "MCP shutdown aborted non-cooperative tasks after timeout");
            }
        }
        self.ports
            .admin
            .shutdown()
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    pub async fn handle(
        &self,
        connection: &super::server::ConnectionContext,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("").to_string();
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        if matches!(
            method.as_str(),
            "session.resume" | "session.fork" | "session.interrupt" | "session.replay"
        ) {
            let session_id = fabric::SessionId(
                params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            let result: anyhow::Result<serde_json::Value> = match method.as_str() {
                "session.resume" => self.ports.turn.session_resume(session_id.clone()).await.map(|resume| serde_json::json!({
                    "session": resume.session, "next_sequence": resume.next_sequence, "messages": resume.messages,
                })),
                "session.fork" => async {
                    let parent_key = crate::application::thread_authority::ThreadAuthorityKey::new(
                        connection.principal_id.clone(),
                        fabric::ThreadId(session_id.0.clone()),
                    );
                    let settings = self
                        .thread_authority
                        .get(&parent_key)
                        .map_err(anyhow::Error::from)?
                        .ok_or_else(|| anyhow::anyhow!("no host-bound authority for parent session"))?;
                    let record = self
                        .ports
                        .turn
                        .session_fork(
                            session_id.clone(),
                            params
                                .get("through_sequence")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                        )
                        .await?;
                    let child_key = crate::application::thread_authority::ThreadAuthorityKey::new(
                        connection.principal_id.clone(),
                        fabric::ThreadId(record.id.0.clone()),
                    );
                    self.thread_authority
                        .bind_or_verify(&child_key, &settings)
                        .map_err(anyhow::Error::from)?;
                    serde_json::to_value(record).map_err(Into::into)
                }
                .await,
                "session.interrupt" => self.ports.turn.session_interrupt(session_id.clone()).await.map(|outcome| serde_json::json!({
                    "outcome": format!("{outcome:?}").to_lowercase(),
                })),
                "session.replay" => self.ports.turn.session_replay(
                    session_id,
                    params.get("after_sequence").and_then(|v| v.as_u64()),
                ).await.map(|messages| serde_json::json!({"messages": messages})),
                _ => unreachable!(),
            };
            return match result {
                Ok(result) => serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err(error) => {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":error.to_string()}})
                }
            };
        }

        // Route session.* methods to the Session Gateway (new unified facade).
        if method.starts_with("session.") {
            if let Some(response) = self
                .ports
                .session_gateway
                .handle_method(&method, &id, &params)
                .await
            {
                return response;
            }
        }

        // Route debug.* methods to the debug handler (backward compat).
        if method.starts_with("debug.") {
            if let Some(response) = self.ports.debug.handle_method(&method, &id, &params).await {
                return response;
            }
        }

        match method.as_str() {
            "client.intent" => self.handle_client_intent(connection, id, params).await,
            "chat" => self.handle_chat(connection, id, request).await,
            _ => self.handle_rpc(connection, &method, id, request).await,
        }
    }

    async fn handle_client_intent(
        &self,
        connection: &super::server::ConnectionContext,
        id: serde_json::Value,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let intent = match serde_json::from_value::<fabric::contract::command::ClientIntent>(params)
        {
            Ok(intent) => intent,
            Err(error) => return rpc_error(&id, -32602, format!("invalid ClientIntent: {error}")),
        };
        self.dispatch_client_intent(connection, id, intent).await
    }

    async fn dispatch_client_intent(
        &self,
        connection: &super::server::ConnectionContext,
        id: serde_json::Value,
        intent: fabric::contract::command::ClientIntent,
    ) -> serde_json::Value {
        if intent.principal != connection.principal_id {
            return rpc_error(
                &id,
                -32602,
                "ClientIntent principal does not match the authenticated transport principal",
            );
        }
        if let Err(error) = intent.validate() {
            return rpc_error(&id, -32602, error.to_string());
        }
        let dispatcher = CommandDispatcher::new(Arc::new(DaemonCommandUseCases {
            handler: self.clone(),
            connection: connection.clone(),
            rpc_id: id.clone(),
        }));
        match dispatcher.dispatch(intent).await {
            Ok(output) => match &output.output {
                fabric::contract::command::CommandOutputV1::Rejected(rejection) => {
                    rpc_error(&id, rejection.code, rejection.message.clone())
                }
                _ => match serde_json::to_value(output) {
                    Ok(result) => serde_json::json!({"jsonrpc":"2.0", "id":id, "result":result}),
                    Err(error) => rpc_error(&id, -32603, error.to_string()),
                },
            },
            Err(error) => rpc_error(&id, -32603, error.to_string()),
        }
    }

    pub(super) async fn handle_legacy_status(
        &self,
        connection: &super::server::ConnectionContext,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let session_id = request["params"]
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| fabric::SessionId(value.to_owned()));
        let intent = fabric::contract::command::ClientIntent::v1(
            fabric::contract::command::ClientSurface::Tui,
            connection.principal_id.clone(),
            format!("legacy-status:{}", rpc_id_fragment(&id)),
            fabric::contract::command::ClientCommand::Status(
                fabric::contract::command::StatusIntent { session_id },
            ),
        );
        // Sole V1→V0 compatibility adapter for the historical `status` RPC.
        // New `client_intent` callers receive CommandOutputEnvelopeV1 directly.
        let dispatcher = CommandDispatcher::new(Arc::new(DaemonCommandUseCases {
            handler: self.clone(),
            connection: connection.clone(),
            rpc_id: id.clone(),
        }));
        match dispatcher.dispatch(intent).await {
            Ok(output) => match output.output {
                fabric::contract::command::CommandOutputV1::StatusProjected(status) => {
                    serde_json::json!({"jsonrpc":"2.0", "id":id, "result":{"status":status}})
                }
                fabric::contract::command::CommandOutputV1::Rejected(rejection) => {
                    rpc_error(&id, rejection.code, rejection.message)
                }
                _ => rpc_error(
                    &id,
                    -32603,
                    "status command returned an incompatible typed output",
                ),
            },
            Err(error) => rpc_error(&id, -32603, error.to_string()),
        }
    }

    /// Thin delegation to the macro-kernel turn orchestrator.
    pub(super) async fn handle_chat(
        &self,
        connection: &super::server::ConnectionContext,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let message = request["params"]["message"].as_str().unwrap_or("");
        let requirements = match parse_turn_requirements(&request["params"]["requirements"]) {
            Ok(requirements) => requirements,
            Err(error) => return rpc_error(&id, -32602, error),
        };
        let task_kind = match parse_task_kind(&request["params"]["task_kind"]) {
            Ok(task_kind) => task_kind,
            Err(error) => return rpc_error(&id, -32602, error),
        };
        let permission_mode = parse_host_permission_mode(&request["params"]["permission_mode"]);
        let workspace =
            match resolve_requested_workspace_with_mode(&request["params"], permission_mode) {
                Ok(workspace) => workspace,
                Err(error) => {
                    return serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": error }
                    });
                }
            };
        let thread_id = if let Some(session_id) = request["params"]["session_id"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        {
            fabric::ThreadId(session_id.to_owned())
        } else {
            match self.select_workspace_session(workspace.cwd()).await {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    return serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": error.to_string() }
                    })
                }
            }
        };
        let intent = fabric::contract::command::ClientIntent::v1(
            fabric::contract::command::ClientSurface::Tui,
            connection.principal_id.clone(),
            format!("legacy-chat:{}", rpc_id_fragment(&id)),
            fabric::contract::command::ClientCommand::SubmitPrompt(
                fabric::contract::command::SubmitPromptIntent {
                    content: message.to_owned(),
                    session_id: Some(fabric::SessionId(thread_id.0)),
                    workspace,
                    requirements,
                    task_kind,
                    permission_mode,
                    execution_target: fabric::ExecutionTargetSelection::default(),
                },
            ),
        );
        self.dispatch_client_intent(connection, id, intent).await
    }

    /// Versioned chat boundary. `thread_id` is protocol data in its own right;
    /// unlike the legacy adapter it is never selected from the workspace cwd.
    pub(crate) async fn execute_explicit_chat(
        &self,
        connection: &super::server::ConnectionContext,
        id: serde_json::Value,
        message: String,
        thread_id: fabric::ThreadId,
        workspace: fabric::WorkspacePolicy,
        requirements: Vec<fabric::TurnRequirement>,
        task_kind: Option<fabric::TaskKind>,
        execution_target: fabric::ExecutionTargetSelection,
        permission_mode: fabric::permission::HostPermissionMode,
    ) -> serde_json::Value {
        if thread_id.0.trim().is_empty() || message.trim().is_empty() {
            return rpc_error(&id, -32602, "thread_id and message are required");
        }
        let mut context = fabric::PrincipalContext::new(
            connection.principal_id.clone(),
            connection.os_principal,
            connection.connection_id.clone(),
            thread_id,
            workspace,
            if permission_mode.is_full() {
                fabric::PermissionProfileId::danger_full_access()
            } else {
                fabric::PermissionProfileId::workspace_write()
            },
            if permission_mode.is_full() {
                fabric::ApprovalPolicy::Never
            } else {
                fabric::ApprovalPolicy::OnRequest
            },
        );
        if let Err(error) = self.bind_thread_authority(&context, None) {
            return serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": error.to_string() }
            });
        }
        let trust_decision = self
            .workspace_trust
            .evaluate(
                context.principal_id.clone(),
                crate::application::workspace_trust::workspace_identity(context.workspace.cwd()),
                fabric::workspace_trust::ClientMode::Headless,
                crate::application::workspace_trust::is_broad_unrecordable_root(
                    context.workspace.cwd(),
                ),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )
            .await;
        context.repo_hooks_trusted = crate::application::workspace_trust::source_is_granted(
            &trust_decision,
            fabric::workspace_trust::ExecutableConfigSource::RepoHooks,
        );
        tracing::debug!(
            principal = %context.principal_id.0,
            workspace = %context.workspace.cwd().display(),
            decision = ?trust_decision,
            "evaluated repository executable configuration trust"
        );
        tracing::info!(message = %message, thread_id = %context.thread_id.0, "Chat request received");
        self.ports
            .turn
            .execute(
                id,
                message,
                context,
                requirements,
                task_kind,
                execution_target,
                self.notify_tx.clone(),
            )
            .await
    }

    /// Keep local conversation history scoped to its canonical workspace.
    /// Without this, a TUI launched in one checkout inherits tool paths from
    /// the last TUI that happened to use the daemon's global default session.
    /// Select the authoritative workspace-scoped Session/Thread for protocol
    /// composition roots (Unix JSON-RPC, ACP, and future edge adapters).
    pub async fn select_workspace_session(
        &self,
        working_dir: &Path,
    ) -> anyhow::Result<fabric::ThreadId> {
        let session_id = self
            .ports
            .sessions
            .route_workspace(working_dir.to_path_buf())
            .await?;
        tracing::info!(%session_id, cwd = %working_dir.display(), "Selected new workspace session");
        Ok(LegacySessionThreadAdapter::thread_id(session_id))
    }

    fn bind_thread_authority(
        &self,
        context: &fabric::PrincipalContext,
        model_policy: Option<String>,
    ) -> Result<(), crate::application::thread_authority::ThreadAuthorityError> {
        use crate::application::thread_authority::{ThreadAuthorityKey, ThreadSettings};
        let key = ThreadAuthorityKey::new(context.principal_id.clone(), context.thread_id.clone());
        self.thread_authority
            .bind_or_verify(&key, &ThreadSettings::from_context(context, model_policy))
    }
}

fn take_rpc_result(response: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("command execution failed");
        anyhow::bail!(message.to_owned());
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("command handler returned no result"))
}

fn rpc_error_parts(response: &serde_json::Value) -> Option<(i64, String)> {
    let error = response.get("error")?;
    Some((
        error
            .get("code")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-32603),
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("command execution failed")
            .to_owned(),
    ))
}

fn rpc_id_fragment(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn parse_turn_requirements(
    value: &serde_json::Value,
) -> Result<Vec<fabric::TurnRequirement>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let requirements: Vec<fabric::TurnRequirement> = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid turn requirements: {error}"))?;
    if requirements.len() > 16 {
        return Err("at most 16 turn requirements are allowed".into());
    }
    for requirement in &requirements {
        let value = match requirement {
            fabric::TurnRequirement::InvokeAgentRuntime { runtime_id } => runtime_id,
            fabric::TurnRequirement::InvokeCapability { name } => name,
            fabric::TurnRequirement::ObserveTerminal { .. } => continue,
            fabric::TurnRequirement::RunRoleGraph {
                workspace_scope,
                expected_evidence,
                ..
            } => {
                if workspace_scope.len() > 64 || expected_evidence.len() > 64 {
                    return Err("role graph requirement lists are limited to 64 items".into());
                }
                if workspace_scope
                    .iter()
                    .chain(expected_evidence)
                    .any(|item| item.trim().is_empty() || item.len() > 4096)
                {
                    return Err("role graph requirement values must contain 1..=4096 bytes".into());
                }
                continue;
            }
        };
        if value.trim().is_empty() || value.len() > 512 {
            return Err("turn requirement identifiers must contain 1..=512 bytes".into());
        }
    }
    Ok(requirements)
}

fn parse_task_kind(value: &serde_json::Value) -> Result<Option<fabric::TaskKind>, String> {
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| format!("invalid task kind: {error}"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn rpc_error(id: &serde_json::Value, code: i64, message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}

/// M0 bridge from the legacy workspace/session router to explicit turn authority.
struct LegacySessionThreadAdapter;

impl LegacySessionThreadAdapter {
    fn thread_id(session_id: String) -> fabric::ThreadId {
        fabric::ThreadId(session_id)
    }
}

fn resolve_requested_workspace(
    params: &serde_json::Value,
) -> Result<fabric::WorkspacePolicy, String> {
    resolve_requested_workspace_with_mode(params, fabric::permission::HostPermissionMode::Safe)
}

fn resolve_requested_workspace_with_mode(
    params: &serde_json::Value,
    permission_mode: fabric::permission::HostPermissionMode,
) -> Result<fabric::WorkspacePolicy, String> {
    let requested = params["working_dir"]
        .as_str()
        .ok_or_else(|| "missing working_dir".to_string())?;
    let roots = params["workspace_roots"]
        .as_array()
        .ok_or_else(|| "missing workspace_roots".to_string())?;
    let roots: Vec<PathBuf> = roots
        .iter()
        .map(|root| {
            root.as_str()
                .map(PathBuf::from)
                .ok_or_else(|| "workspace_roots must contain only paths".to_string())
        })
        .collect::<Result<_, _>>()?;
    if roots.first().map(PathBuf::as_path) != Some(Path::new(requested)) {
        return Err("working_dir must be the first workspace root".into());
    }
    fabric::WorkspaceSelection::new(
        Some(PathBuf::from(requested)),
        roots.into_iter().skip(1).collect(),
    )
    .resolve_with_profile(
        Path::new(requested),
        &if permission_mode.is_full() {
            fabric::PermissionProfileId::danger_full_access()
        } else {
            fabric::PermissionProfileId::workspace_write()
        },
    )
    .map_err(|error| error.to_string())
}

fn parse_host_permission_mode(value: &serde_json::Value) -> fabric::permission::HostPermissionMode {
    match value.as_str() {
        Some("full" | "unrestricted") => fabric::permission::HostPermissionMode::Full,
        Some("developer" | "dev") => fabric::permission::HostPermissionMode::Developer,
        _ => fabric::permission::HostPermissionMode::Safe,
    }
}

#[cfg(test)]
mod working_dir_tests {
    #[test]
    fn rejects_workspace_without_roots() {
        assert!(
            super::resolve_requested_workspace(&serde_json::json!({"working_dir":"/tmp"})).is_err()
        );
    }

    #[test]
    fn rejects_missing_local_working_directory() {
        assert!(super::resolve_requested_workspace(&serde_json::json!({"working_dir":"/does-not-exist","workspace_roots":["/does-not-exist"]})).is_err());
    }

    #[test]
    fn accepts_canonical_workspace_roots() {
        let root = std::env::temp_dir().join(format!("aletheon-cwd-test-{}", std::process::id()));
        let project = root.join("aletheon");
        std::fs::create_dir_all(&project).unwrap();
        let workspace = super::resolve_requested_workspace(&serde_json::json!({
            "working_dir": project,
            "workspace_roots": [project, root]
        }))
        .unwrap();
        assert_eq!(workspace.writable_roots().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_typed_agent_runtime_requirement() {
        let requirements = super::parse_turn_requirements(&serde_json::json!([
            {"InvokeAgentRuntime":{"runtime_id":"pi-rpc"}}
        ]))
        .unwrap();
        assert_eq!(
            requirements,
            vec![fabric::TurnRequirement::InvokeAgentRuntime {
                runtime_id: "pi-rpc".into()
            }]
        );
    }

    #[test]
    fn rejects_empty_requirement_identifier() {
        assert!(super::parse_turn_requirements(&serde_json::json!([
            {"InvokeAgentRuntime":{"runtime_id":"  "}}
        ]))
        .is_err());
    }

    #[test]
    fn task_kind_parser_accepts_only_typed_coding_value() {
        assert_eq!(
            super::parse_task_kind(&serde_json::json!("coding")).unwrap(),
            Some(fabric::TaskKind::Coding)
        );
        assert!(super::parse_task_kind(&serde_json::json!("write code")).is_err());
        assert_eq!(
            super::parse_task_kind(&serde_json::Value::Null).unwrap(),
            None
        );
    }

    #[test]
    fn permission_mode_parser_is_fail_closed() {
        assert!(super::parse_host_permission_mode(&serde_json::json!("full")).is_full());
        assert_eq!(
            super::parse_host_permission_mode(&serde_json::json!("dev")),
            fabric::permission::HostPermissionMode::Developer
        );
        assert_eq!(
            super::parse_host_permission_mode(&serde_json::json!("unknown")),
            fabric::permission::HostPermissionMode::Safe
        );
    }
}
