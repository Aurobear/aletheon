//! ACP executable composition root.
//!
//! This binary layer may depend on both Executive and Interact. Neither crate
//! needs to depend on the other: Interact owns translation while Executive's
//! existing authenticated request handler remains the runtime authority.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use executive::{core::runtime_core::RuntimeCore, r#impl::daemon::server::ConnectionContext};
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId, PrincipalContext,
    PrincipalId, ThreadId, WorkspacePolicy,
};
use interact::acp::{
    run_transport_loop, AcpAdapter, AcpBackend, AcpError, AcpEventSource, AcpSessionEvent,
    AuthenticatedAcpConnection, CreatedAcpSession,
};
use tokio::{
    io::BufReader,
    sync::{mpsc, Mutex},
};

use executive::host::launcher::WorkspaceLaunch;

struct ExecutiveAcpBackend {
    handler: executive::r#impl::daemon::handler::RequestHandler,
    connection: ConnectionContext,
    active_session: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl AcpBackend for ExecutiveAcpBackend {
    async fn create_session(
        &self,
        principal: &PrincipalContext,
        cwd: &Path,
    ) -> Result<CreatedAcpSession, AcpError> {
        verify_principal(principal, &self.connection)?;
        let thread_id = self
            .handler
            .select_workspace_session(cwd)
            .await
            .map_err(|error| AcpError::Backend(error.to_string()))?;
        let session_id = thread_id.0.clone();
        *self.active_session.lock().await = Some(session_id.clone());
        Ok(CreatedAcpSession {
            thread_id,
            session_id,
        })
    }

    async fn submit_prompt(
        &self,
        principal: &PrincipalContext,
        session_id: &str,
        _thread_id: &ThreadId,
        text: &str,
    ) -> Result<(), AcpError> {
        verify_principal(principal, &self.connection)?;
        *self.active_session.lock().await = Some(session_id.to_string());
        let handler = self.handler.clone();
        let connection = self.connection.clone();
        let workspace = principal.workspace.cwd().to_string_lossy().into_owned();
        let text = text.to_string();
        tokio::spawn(async move {
            let response = handler
                .handle(
                    &connection,
                    serde_json::json!({
                        "jsonrpc":"2.0", "id":2, "method":"chat",
                        "params":{"message":text,"cwd":workspace}
                    }),
                )
                .await;
            if response.get("error").is_some() {
                tracing::warn!(event="acp.prompt.failed", error=?response["error"], "ACP prompt failed in Executive");
            }
        });
        Ok(())
    }

    async fn cancel_turn(
        &self,
        principal: &PrincipalContext,
        session_id: &str,
        _thread_id: &ThreadId,
    ) -> Result<(), AcpError> {
        verify_principal(principal, &self.connection)?;
        let response = self
            .handler
            .handle(
                &self.connection,
                serde_json::json!({
                    "jsonrpc":"2.0", "id":3, "method":"session.interrupt",
                    "params":{"session_id":session_id}
                }),
            )
            .await;
        rpc_result(&response)?;
        Ok(())
    }
}

struct ExecutiveEvents {
    receiver: mpsc::Receiver<String>,
    active_session: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl AcpEventSource for ExecutiveEvents {
    async fn next_event(&mut self) -> Result<Option<AcpSessionEvent>, AcpError> {
        while let Some(frame) = self.receiver.recv().await {
            let value: serde_json::Value = serde_json::from_str(&frame)
                .map_err(|error| AcpError::Backend(error.to_string()))?;
            if value.get("method").and_then(|v| v.as_str()) != Some("event") {
                tracing::debug!(
                    event = "acp.event.unmapped",
                    "ignored non-event Executive notification"
                );
                continue;
            }
            let event = serde_json::from_value(value["params"].clone())
                .map_err(|error| AcpError::Backend(error.to_string()))?;
            let Some(session_id) = self.active_session.lock().await.clone() else {
                tracing::warn!(
                    event = "acp.event.unmapped",
                    "Executive event arrived before ACP session binding"
                );
                continue;
            };
            return Ok(Some(AcpSessionEvent { session_id, event }));
        }
        Ok(None)
    }
}

pub async fn run(workspace: WorkspaceLaunch) -> Result<()> {
    let mut core = RuntimeCore::bootstrap(None, false).await?;
    anyhow::ensure!(
        core.app_config.grok_hardening.acp_adapter,
        "ACP entry is disabled; enable grok_hardening.acp_adapter"
    );
    let policy = resolve_workspace(workspace)?;
    let os_principal = authenticated_process_principal()?;
    let connection_id = ConnectionId::new();
    let connection = ConnectionContext {
        principal_id: PrincipalId::local_uid(os_principal.uid),
        os_principal,
        connection_id: connection_id.clone(),
    };
    let principal = interact::acp::establish_principal(
        os_principal,
        connection_id,
        ThreadId("acp-stdio".into()),
        policy,
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    );
    let authenticated = AuthenticatedAcpConnection::new(principal);
    let receiver = core.request_handler.create_notify_channel().await;
    let active_session = Arc::new(Mutex::new(None));
    let backend = ExecutiveAcpBackend {
        handler: core.request_handler.clone(),
        connection,
        active_session: active_session.clone(),
    };
    let mut events = ExecutiveEvents {
        receiver,
        active_session,
    };
    let mut adapter = AcpAdapter::default();
    tracing::info!(
        event = "acp.gateway.started",
        uid = os_principal.uid,
        "ACP stdio gateway started"
    );
    let stdin = BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    let mut transport = interact::acp::transport::AcpTransport::new(stdin, stdout);
    let result = run_transport_loop(
        &mut adapter,
        &authenticated,
        &backend,
        &mut events,
        &mut transport,
    )
    .await;
    core.cancel_token.cancel();
    core.request_handler.shutdown_runtime().await?;
    result.context("ACP stdio transport")
}

fn authenticated_process_principal() -> Result<LocalOsPrincipal> {
    // In stdio mode there is no peer socket. Kernel process credentials are the
    // only authenticated identity source; there is deliberately no env/client fallback.
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    anyhow::ensure!(
        uid != u32::MAX && gid != u32::MAX,
        "kernel credentials unavailable"
    );
    Ok(LocalOsPrincipal { uid, gid })
}

fn resolve_workspace(workspace: WorkspaceLaunch) -> Result<WorkspacePolicy> {
    let cwd = std::fs::canonicalize(workspace.cwd.unwrap_or(std::env::current_dir()?))?;
    anyhow::ensure!(
        cwd.is_dir() && cwd != Path::new("/"),
        "ACP workspace must be a non-root directory"
    );
    let extra = workspace
        .add_dirs
        .into_iter()
        .map(std::fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()?;
    WorkspacePolicy::from_resolved_roots(cwd, extra).map_err(anyhow::Error::msg)
}

fn verify_principal(
    principal: &PrincipalContext,
    connection: &ConnectionContext,
) -> Result<(), AcpError> {
    if principal.principal_id != connection.principal_id
        || principal.os_principal != connection.os_principal
        || principal.connection_id != connection.connection_id
    {
        return Err(AcpError::Backend("authenticated principal mismatch".into()));
    }
    Ok(())
}

fn rpc_result(response: &serde_json::Value) -> Result<&serde_json::Value, AcpError> {
    if let Some(error) = response.get("error") {
        return Err(AcpError::Backend(error.to_string()));
    }
    response
        .get("result")
        .ok_or_else(|| AcpError::Backend("Executive RPC omitted result".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_mismatch_fails_closed_before_executive_call() {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            WorkspacePolicy::from_resolved_roots(root.path().to_path_buf(), vec![]).unwrap();
        let principal = interact::acp::establish_principal(
            LocalOsPrincipal {
                uid: 1000,
                gid: 1000,
            },
            ConnectionId::new(),
            ThreadId("t".into()),
            workspace,
            PermissionProfileId::workspace_write(),
            ApprovalPolicy::OnRequest,
        );
        let connection = ConnectionContext {
            principal_id: PrincipalId::local_uid(2000),
            os_principal: LocalOsPrincipal {
                uid: 2000,
                gid: 2000,
            },
            connection_id: principal.connection_id.clone(),
        };
        assert!(verify_principal(&principal, &connection).is_err());
    }

    #[test]
    fn rpc_error_never_becomes_success() {
        let response = serde_json::json!({"error":{"code":-32000,"message":"denied"}});
        assert!(rpc_result(&response).is_err());
    }
}
