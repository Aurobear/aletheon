//! execute_turn — the main orchestration entry point for daemon chat turns.
//!
//! This method handles kernel registration (process table, operation table) then
//! delegates the full Pre/Cognit/Post pipeline to `TurnPipeline::run()`.

use super::orchestrator::DaemonTurnOrchestrator;

use crate::service::turn_coordinator::TurnExecution;
use crate::service::turn_policy::TurnPolicy;
use fabric::{PrincipalId, TurnRequest, TurnResult, TurnStop};
use serde_json::json;
use tracing::warn;

impl DaemonTurnOrchestrator {
    /// Execute a full daemon chat turn through the macro-kernel pipeline.
    ///
    /// Returns the JSON-RPC response value. This replaces the body of
    /// `RequestHandler::handle_chat`.
    pub async fn execute_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        working_dir: std::path::PathBuf,
    ) -> serde_json::Value {
        self.execute_turn_for_principal(id, message, None, working_dir)
            .await
    }

    /// Execute a channel turn under an identity established by the channel
    /// binding. The principal never comes from model-visible input.
    pub async fn execute_authenticated_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        principal: PrincipalId,
    ) -> serde_json::Value {
        self.execute_turn_for_principal(
            id,
            message,
            Some(principal),
            std::path::PathBuf::from("/var/lib/aletheon"),
        )
        .await
    }

    async fn execute_turn_for_principal(
        &self,
        id: serde_json::Value,
        message: &str,
        principal: Option<PrincipalId>,
        working_dir: std::path::PathBuf,
    ) -> serde_json::Value {
        // -- Kernel: register main agent --
        let main_pid = match self.ensure_main_agent().await {
            Ok(pid) => pid,
            Err(e) => {
                warn!(error = %e, "Failed to register main agent in process table");
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("Kernel error: {e}")}});
            }
        };

        let session_id = self
            .subsystems
            .session
            .default_session_id
            .lock()
            .await
            .clone();
        let principal = principal.unwrap_or_else(|| PrincipalId(session_id.clone()));

        // The coordinator replaces this placeholder with the authoritative Turn id.
        let turn_request = TurnRequest {
            operation_id: fabric::OperationId::default(),
            process_id: main_pid,
            session_id: session_id.clone(),
            input: message.to_string(),
            working_dir,
            model_policy: None,
            deadline: None,
        };

        let _turn_token = self.begin_turn_token().await;
        let pipeline = self.pipeline.clone();
        let rpc_id = id.clone();
        let policy = TurnPolicy::daemon();
        let coordinated = self
            .coordinator
            .submit_with(turn_request, &policy, move |request, cancel| async move {
                let turn_cancel = cancel.clone();
                {
                    let mut guard = pipeline.current_scope.lock().await;
                    *guard = Some(aletheon_kernel::operation::OperationScope::new(
                        request.operation_id,
                    ));
                }
                let response = pipeline
                    .run(
                        rpc_id,
                        request.input.clone(),
                        request.clone(),
                        request.operation_id,
                        request.process_id,
                        cancel,
                        principal,
                    )
                    .await?;
                if let Some(error) = response.get("error") {
                    anyhow::bail!(
                        "{}",
                        error
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("daemon turn failed")
                    );
                }
                let result = &response["result"];
                let output = result["response"].as_str().unwrap_or_default().to_string();
                let items =
                    serde_json::from_value(result["canonical_items"].clone()).unwrap_or_default();
                let succeeded = result["succeeded"].as_bool().unwrap_or(false);
                let metric = &result["metrics"];
                let metrics = fabric::TurnMetrics {
                    tool_calls_made: metric["tool_calls_made"].as_u64().unwrap_or(0) as usize,
                    tool_errors: metric["tool_errors"].as_u64().unwrap_or(0) as usize,
                    elapsed_ms: metric["elapsed_ms"].as_u64().unwrap_or(0),
                    iterations: metric["iterations"].as_u64().unwrap_or(0) as usize,
                    completed_normally: metric["completed_normally"].as_bool().unwrap_or(false),
                };
                Ok(TurnExecution {
                    result: TurnResult {
                        output,
                        stop: if turn_cancel.is_cancelled() {
                            TurnStop::Cancelled
                        } else if succeeded {
                            TurnStop::Completed
                        } else {
                            TurnStop::Failed
                        },
                        metrics,
                    },
                    items,
                })
            })
            .await;
        match coordinated {
            Ok(result) => {
                json!({"jsonrpc": "2.0", "id": id, "result": {"response": result.output}})
            }
            Err(error) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}})
            }
        }
    }
}
