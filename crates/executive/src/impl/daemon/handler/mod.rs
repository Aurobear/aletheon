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

pub(crate) async fn run_hook_scripts(scripts: &[String], input_json: &str) {
    for script_path in scripts {
        let path = format::expand_tilde(script_path);
        if !std::path::Path::new(&path).exists() {
            tracing::warn!(path = %path, "Hook script not found, skipping");
            continue;
        }
        let spawn_result = tokio::process::Command::new(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match spawn_result {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(path = %path, %error, "Failed to spawn hook script");
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input_json.as_bytes()).await;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await {
            Ok(Ok(status)) if status.success() => {}
            Ok(Ok(status)) => {
                tracing::warn!(path = %path, code = status.code(), "Hook script failed")
            }
            Ok(Err(error)) => tracing::warn!(path = %path, %error, "Hook script I/O error"),
            Err(_) => {
                tracing::warn!(path = %path, "Hook script timed out");
                let _ = child.kill().await;
            }
        }
    }
}

#[derive(Clone)]
pub struct RequestHandler {
    /// Narrow application use cases available to protocol handlers.
    pub(crate) ports: Arc<ports::HandlerPorts>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
}

impl RequestHandler {
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
            if let Some(response) = self
                .ports
                .debug
                .handler()
                .handle_method(&method, &id, &params)
                .await
            {
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
        let thread_id = match self.select_workspace_session(workspace.cwd()).await {
            Ok(thread_id) => thread_id,
            Err(error) => {
                return serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32603, "message": error.to_string() }
                })
            }
        };
        let context = fabric::PrincipalContext::new(
            connection.principal_id.clone(),
            connection.os_principal,
            connection.connection_id.clone(),
            thread_id,
            workspace,
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
        );
        if let Err(error) = bind_thread_authority(&context, None) {
            return serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": error.to_string() }
            });
        }
        tracing::info!(message = %message, "Chat request received");
        self.ports
            .turn
            .execute(id, message.to_owned(), context)
            .await
    }

    /// Keep local conversation history scoped to its canonical workspace.
    /// Without this, a TUI launched in one checkout inherits tool paths from
    /// the last TUI that happened to use the daemon's global default session.
    async fn select_workspace_session(
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

fn bind_thread_authority(
    context: &fabric::PrincipalContext,
    model_policy: Option<String>,
) -> Result<(), crate::service::thread_authority::ThreadAuthorityError> {
    use crate::service::thread_authority::{
        ThreadAuthorityKey, ThreadAuthorityStore, ThreadSettings,
    };
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or(ThreadAuthorityError::StateRootUnavailable)?;
    let store = ThreadAuthorityStore::persistent(state_home.join("aletheon/thread-authority"));
    let key = ThreadAuthorityKey::new(context.principal_id.clone(), context.thread_id.clone());
    store.bind_or_verify(&key, &ThreadSettings::from_context(context, model_policy))
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
