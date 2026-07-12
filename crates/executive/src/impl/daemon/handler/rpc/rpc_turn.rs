//! Turn lifecycle RPC handlers — wait / cancel / exit (PR-3).
//!
//! Methods: turn.wait, turn.cancel, turn.exit.

use super::RequestHandler;

use serde_json::json;
use tracing::{info, warn};

impl RequestHandler {
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

        match self.turn_orchestrator.wait_turn(operation_id).await {
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

        match self.turn_orchestrator.cancel_turn(operation_id).await {
            Ok(()) => {
                info!(?operation_id, "turn.cancel succeeded");
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
    /// Delegates to the kernel ProcessTable. The process transitions through
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

        match self.turn_orchestrator.exit_process(process_id).await {
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
