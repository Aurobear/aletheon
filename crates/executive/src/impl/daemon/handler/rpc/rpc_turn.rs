//! Turn lifecycle RPC handlers — wait / cancel / exit (PR-3).
//!
//! Methods: turn.wait, turn.cancel, turn.exit.

use super::RequestHandler;

use serde_json::json;
use tracing::{info, warn};

fn parse_workspace_rewind_request(params: &serde_json::Value) -> Result<(&str, u64), &'static str> {
    if params.get("working_dir").is_some()
        || params.get("workspace_roots").is_some()
        || params.get("checkpoint").is_some()
        || params.get("checkpoint_blob").is_some()
        || params.get("checkpoint_path").is_some()
    {
        return Err("workspace paths and checkpoint data are host-controlled");
    }
    let session_id = params
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("session_id and prompt_index are required")?;
    let prompt_index = params
        .get("prompt_index")
        .and_then(serde_json::Value::as_u64)
        .ok_or("session_id and prompt_index are required")?;
    Ok((session_id, prompt_index))
}

impl RequestHandler {
    /// Explicit user-triggered FS rewind. The caller supplies only the logical
    /// session/turn index. Workspace identity is resolved from immutable
    /// host-bound thread authority; no path or checkpoint data is accepted.
    pub(super) async fn handle_workspace_rewind(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let (session_id, prompt_index) = match parse_workspace_rewind_request(&request["params"]) {
            Ok(parsed) => parsed,
            Err(error) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                        "error": { "code": -32602, "message": error }
                });
            }
        };
        let authority_key = crate::service::thread_authority::ThreadAuthorityKey::new(
            connection.principal_id.clone(),
            fabric::ThreadId(session_id.to_owned()),
        );
        let workspace = match self.thread_authority.get(&authority_key) {
            Ok(Some(settings)) => settings.workspace,
            Ok(None) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32044, "message": "no host-bound workspace authority for session" }
                });
            }
            Err(error) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32603, "message": error.to_string() }
                });
            }
        };
        let outcome = self
            .ports
            .turn
            .rewind_workspace(
                connection.principal_id.clone(),
                session_id.to_owned(),
                prompt_index,
                fabric::types::workspace_checkpoint::WorkspaceIdentity {
                    canonical_path: workspace.cwd().to_path_buf(),
                    repo_fingerprint: None,
                },
            )
            .await;
        if outcome == fabric::types::workspace_checkpoint::RestoreOutcome::Completed {
            json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32043, "message": "workspace rewind did not complete", "data": outcome }
            })
        }
    }

    /// Wait for a turn operation to reach a terminal state.
    ///
    /// JSON-RPC params:
    ///   operation_id: string (UUID) — the operation to wait on.
    pub(super) async fn handle_turn_wait(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let operation_id_str = request["params"]["operation_id"].as_str().unwrap_or("");
        if operation_id_str.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing operation_id parameter" }
            });
        }
        let operation_id = match uuid::Uuid::parse_str(operation_id_str) {
            Ok(u) => fabric::OperationId(u),
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": format!("Invalid operation_id UUID: {e}") }
                });
            }
        };

        match self.ports.turn.wait(operation_id).await {
            Ok(result) => {
                info!(?operation_id, state = ?result.state, "turn.wait completed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "operation_id": operation_id_str,
                        "state": format!("{:?}", result.state),
                        "exit": result.exit.map(|e| format!("{:?}", e)),
                    }
                })
            }
            Err(e) => {
                warn!(error = %e, "turn.wait failed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32040, "message": format!("wait failed: {e}") }
                })
            }
        }
    }

    /// Cancel an in-flight turn operation.
    ///
    /// JSON-RPC params:
    ///   operation_id: string (UUID) — the operation to cancel.
    ///   thread_id: string (optional) — thread identifier for identity-aware cancel.
    ///   turn_id: string (optional) — turn identifier for identity-aware cancel.
    ///
    /// Cancels the per-turn OperationScope's CancellationToken (cooperative)
    /// and propagates cancellation through the kernel operation tree.
    pub(super) async fn handle_turn_cancel(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let operation_id_str = request["params"]["operation_id"].as_str().unwrap_or("");
        if operation_id_str.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing operation_id parameter" }
            });
        }
        let operation_id = match uuid::Uuid::parse_str(operation_id_str) {
            Ok(u) => fabric::OperationId(u),
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": format!("Invalid operation_id UUID: {e}") }
                });
            }
        };

        let thread_id = request["params"]["thread_id"].as_str().unwrap_or("");
        let turn_id = request["params"]["turn_id"].as_str().unwrap_or("");

        let result = if self.grok_hardening.prompt_queue && !thread_id.is_empty() {
            // G3 identity-aware cancel: parse principal from connection context
            // and use thread_id + operation_id for lookup.
            self.ports
                .turn
                .cancel_by_key(
                    fabric::PrincipalId("local".into()),
                    thread_id.to_string(),
                    operation_id,
                )
                .await
        } else {
            // Legacy cancel: operation_id only.
            self.ports.turn.cancel(operation_id).await
        };

        match result {
            Ok(()) => {
                info!(?operation_id, thread_id, turn_id, "turn.cancel succeeded");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "operation_id": operation_id_str, "status": "cancelled" }
                })
            }
            Err(e) => {
                warn!(error = %e, "turn.cancel failed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32041, "message": format!("cancel failed: {e}") }
                })
            }
        }
    }

    /// Signal a process to exit (Terminate).
    ///
    /// JSON-RPC params:
    ///   process_id: string (UUID) — the process to terminate.
    ///
    /// Delegates to the kernel runtime. The process transitions through
    /// Stopping → Exited/Failed, and in-flight operations are cancelled via
    /// parent-cancel propagation in the operation tree.
    pub(super) async fn handle_turn_exit(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let process_id_str = request["params"]["process_id"].as_str().unwrap_or("");
        if process_id_str.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing process_id parameter" }
            });
        }
        let process_id = match uuid::Uuid::parse_str(process_id_str) {
            Ok(u) => fabric::ProcessId(u),
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": format!("Invalid process_id UUID: {e}") }
                });
            }
        };

        match self.ports.turn.exit(process_id).await {
            Ok(()) => {
                info!(?process_id, "turn.exit succeeded");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "process_id": process_id_str, "status": "terminated" }
                })
            }
            Err(e) => {
                warn!(error = %e, "turn.exit failed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32042, "message": format!("exit failed: {e}") }
                })
            }
        }
    }
}

#[cfg(test)]
mod workspace_rewind_tests {
    use super::parse_workspace_rewind_request;

    #[test]
    fn rewind_accepts_only_logical_session_and_turn() {
        assert_eq!(
            parse_workspace_rewind_request(
                &serde_json::json!({"session_id": "session-a", "prompt_index": 7})
            ),
            Ok(("session-a", 7))
        );
        for forbidden in [
            "working_dir",
            "workspace_roots",
            "checkpoint",
            "checkpoint_blob",
            "checkpoint_path",
        ] {
            let mut params = serde_json::json!({"session_id": "session-a", "prompt_index": 7});
            params[forbidden] = serde_json::json!("attacker-controlled");
            assert!(parse_workspace_rewind_request(&params).is_err());
        }
    }
}
