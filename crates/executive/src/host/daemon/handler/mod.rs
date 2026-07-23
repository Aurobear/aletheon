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

use crate::composition::config::GrokHardeningConfig;

#[derive(Clone)]
pub struct RequestHandler {
    /// Narrow application use cases available to protocol handlers.
    pub(crate) ports: Arc<ports::HandlerPorts>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// User-state-root-scoped immutable thread authority records.
    pub(crate) thread_authority: Arc<crate::application::thread_authority::ThreadAuthorityStore>,
    /// Feature flags for Grok-hardening mechanisms (folder_trust, etc.).
    pub(crate) grok_hardening: GrokHardeningConfig,
    /// Principal-scoped gate for repository-provided executable configuration.
    pub(crate) workspace_trust: Arc<crate::application::workspace_trust::WorkspaceTrustResolver>,
    /// Retained optional MCP runtime for health projection and bounded shutdown.
    pub(crate) mcp: Option<Arc<corpus::tools::mcp::manager::McpManager>>,
}

impl RequestHandler {
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

    pub(crate) async fn protocol_snapshot(
        &self,
        session_id: &fabric::SessionId,
    ) -> anyhow::Result<fabric::protocol::client::UiSnapshot> {
        self.ports
            .session_gateway
            .protocol_snapshot(session_id)
            .await
    }

    pub(crate) async fn protocol_events_after(
        &self,
        session_id: &fabric::SessionId,
        after: &fabric::protocol::client::EventCursor,
    ) -> anyhow::Result<Vec<fabric::protocol::client::ClientEvent>> {
        self.ports
            .session_gateway
            .protocol_events_after(session_id, after)
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
                "session.fork" => self.ports.turn.session_fork(
                    session_id.clone(),
                    params.get("through_sequence").and_then(|v| v.as_u64()).unwrap_or(0),
                ).await.and_then(|record| serde_json::to_value(record).map_err(Into::into)),
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
            "chat" => self.handle_chat(connection, id, request).await,
            _ => self.handle_rpc(connection, &method, id, request).await,
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
        let workspace = match resolve_requested_workspace(&request["params"]) {
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
        self.execute_explicit_chat(connection, id, message.to_owned(), thread_id, workspace)
            .await
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
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
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
        self.ports.turn.execute(id, message, context).await
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
    .resolve(Path::new(requested))
    .map_err(|error| error.to_string())
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
}
