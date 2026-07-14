//! execute_turn — the main orchestration entry point for daemon chat turns.
//!
//! This method handles kernel registration (process table, operation table) then
//! delegates the full Pre/Cognit/Post pipeline to `TurnPipeline::run()`.

use super::orchestrator::DaemonTurnOrchestrator;

use fabric::{OperationKind, OperationManager, OperationRequest, TurnRequest};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::warn;

impl DaemonTurnOrchestrator {
    /// Execute a full daemon chat turn through the macro-kernel pipeline.
    ///
    /// Returns the JSON-RPC response value. This replaces the body of
    /// `RequestHandler::handle_chat`.
    pub async fn execute_turn(&self, id: serde_json::Value, message: &str) -> serde_json::Value {
        // -- Kernel: register main agent --
        let main_pid = match self.ensure_main_agent().await {
            Ok(pid) => pid,
            Err(e) => {
                warn!(error = %e, "Failed to register main agent in process table");
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("Kernel error: {e}")}});
            }
        };

        // -- Kernel: create per-turn operation --
        let operation = match self
            .operation_table
            .submit(OperationRequest {
                owner: main_pid,
                parent: None,
                kind: OperationKind::SubAgent,
                deadline: None,
            })
            .await
        {
            Ok(op) => {
                let _ = self.operation_table.start(op.id).await;
                op
            }
            Err(e) => {
                warn!(error = %e, "Failed to create turn operation");
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("Operation error: {e}")}});
            }
        };

        let session_id = self
            .subsystems
            .session
            .default_session_id
            .lock()
            .await
            .clone();

        // Build TurnRequest with kernel ids
        let turn_request = TurnRequest {
            operation_id: operation.id,
            process_id: main_pid,
            session_id: session_id.clone(),
            input: message.to_string(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            model_policy: None,
            deadline: None,
        };

        // Per-turn cancel token
        let _turn_token = self.begin_turn_token().await;

        // PR-3: OperationScope for structured cancellation of the react task.
        {
            let mut guard = self.pipeline.current_scope.lock().await;
            *guard = Some(aletheon_kernel::operation::OperationScope::new(
                operation.id,
            ));
        }
        let scope_token = {
            let guard = self.pipeline.current_scope.lock().await;
            guard
                .as_ref()
                .map(|s| s.token())
                .unwrap_or_else(CancellationToken::new)
        };

        // Delegate to shared TurnPipeline
        self.pipeline
            .run(
                id.clone(),
                message.to_string(),
                turn_request,
                operation.id,
                main_pid,
                scope_token,
            )
            .await
            .unwrap_or_else(|e| {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": e.to_string()}})
            })
    }
}
