//! execute_turn — the main orchestration entry point for daemon chat turns.
//!
//! This method handles kernel registration (process table, operation table) then
//! delegates the full Pre/Cognit/Post pipeline to `TurnPipeline::run()`.

use super::orchestrator::DaemonTurnOrchestrator;

use crate::service::turn_coordinator::TurnExecution;
use crate::service::turn_policy::TurnPolicy;
use fabric::{PrincipalContext, PromptEnvelope, PromptKind, TurnRequest, TurnResult, TurnStop};
use serde_json::json;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptAdmissionMode {
    Direct,
    Queued,
}

fn prompt_admission_mode(enabled: bool) -> PromptAdmissionMode {
    if enabled {
        PromptAdmissionMode::Queued
    } else {
        PromptAdmissionMode::Direct
    }
}

impl DaemonTurnOrchestrator {
    /// Execute a full daemon chat turn through the macro-kernel pipeline.
    ///
    /// Returns the JSON-RPC response value. This replaces the body of
    /// `RequestHandler::handle_chat`.
    pub async fn execute_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
    ) -> serde_json::Value {
        self.execute_turn_with_context(id, message, context).await
    }

    /// Execute a channel turn under an identity established by the channel
    /// binding. The principal never comes from model-visible input.
    pub async fn execute_authenticated_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
    ) -> serde_json::Value {
        self.execute_turn_with_context(id, message, context).await
    }

    async fn execute_turn_with_context(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
    ) -> serde_json::Value {
        if prompt_admission_mode(self.grok_hardening.prompt_queue) == PromptAdmissionMode::Direct {
            return self.execute_one_turn(id, message, context).await;
        }

        let principal = context.principal_id.clone();
        let thread = context.thread_id.clone();
        let session_input = self.coordinator.session_input();
        let idempotency_key = format!(
            "chat:{}:{}:{}",
            context.connection_id.0,
            thread.0,
            serde_json::to_string(&id).unwrap_or_default()
        );
        let queued = match session_input
            .enqueue(
                principal.clone(),
                context.connection_id.clone(),
                thread.clone(),
                PromptKind::Prompt,
                message.to_owned(),
                idempotency_key,
            )
            .await
        {
            Ok(prompt) => prompt,
            Err(error) => {
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}});
            }
        };

        if !session_input.try_claim_processor(&principal, &thread).await {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "queued": true,
                    "prompt_id": queued.prompt_id.0.to_string()
                }
            });
        }

        let mut requested_result = None;
        loop {
            let next = match session_input
                .take_next_or_release(&principal, &thread)
                .await
            {
                Ok(Some(prompt)) => prompt,
                Ok(None) => break,
                Err(error) => {
                    session_input.release_processor(&principal, &thread).await;
                    return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}});
                }
            };
            let prompt_id = next.prompt_id;
            let rpc_id = if prompt_id == queued.prompt_id {
                id.clone()
            } else {
                serde_json::Value::Null
            };
            let turn_result = self
                .execute_queued_prompt(rpc_id, next, context.clone())
                .await;
            let succeeded = turn_result.get("error").is_none();
            if succeeded {
                let receipt = format!("turn-completed:{prompt_id:?}");
                if let Err(error) = session_input
                    .mark_prompt_completed(prompt_id, &receipt)
                    .await
                {
                    warn!(%error, ?prompt_id, "failed to persist prompt completion");
                }
            } else if let Err(error) = session_input.mark_prompt_rejected(prompt_id).await {
                warn!(%error, ?prompt_id, "failed to persist prompt rejection");
            }
            if prompt_id == queued.prompt_id {
                requested_result = Some(turn_result);
            }
        }
        requested_result.unwrap_or_else(|| {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "queued prompt was not executed"}})
        })
    }

    async fn execute_queued_prompt(
        &self,
        id: serde_json::Value,
        prompt: PromptEnvelope,
        mut context: PrincipalContext,
    ) -> serde_json::Value {
        context.connection_id = prompt.connection_id;
        context.thread_id = prompt.thread_id;
        self.execute_one_turn(id, &prompt.content, context).await
    }

    async fn execute_one_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
    ) -> serde_json::Value {
        // -- Kernel: register main agent --
        let main_pid = match self.ensure_main_agent().await {
            Ok(pid) => pid,
            Err(e) => {
                warn!(error = %e, "Failed to register main agent in process table");
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("Kernel error: {e}")}});
            }
        };

        // The coordinator replaces this placeholder with the authoritative Turn id.
        let turn_request = TurnRequest {
            operation_id: fabric::OperationId::default(),
            process_id: main_pid,
            context,
            input: message.to_string(),
            model_policy: None,
            deadline: None,
        };

        let _turn_token = self.begin_turn_token().await;
        let pipeline = self.pipeline.clone();
        #[cfg(test)]
        let test_runner = self.test_runner.clone();
        let rpc_id = id.clone();
        let principal = turn_request.context.principal_id.clone();
        let policy = TurnPolicy::daemon();
        let coordinated = self
            .coordinator
            .submit_with(turn_request, &policy, move |request, cancel| async move {
                #[cfg(test)]
                if let Some(runner) = test_runner {
                    return runner(request, cancel).await;
                }
                let pipeline =
                    pipeline.expect("production daemon orchestrator has a turn pipeline");
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
                let projection = crate::service::post_turn_projection::PostTurnDispatch {
                    projector: pipeline.post_turn_projection.clone(),
                    outcome: crate::service::post_turn_projection::PostTurnOutcome {
                        session_id: result["projection"]["session_id"]
                            .as_str()
                            .unwrap_or(&request.context.thread_id.0)
                            .to_string(),
                        input: request.input.clone(),
                        output: output.clone(),
                        turn: result["turn"].as_u64().unwrap_or(0) as usize,
                        succeeded,
                        tool_calls_made: metrics.tool_calls_made,
                        tool_errors: metrics.tool_errors,
                        elapsed_ms: metrics.elapsed_ms,
                        iterations: metrics.iterations,
                        completed_normally: metrics.completed_normally,
                        agora_start_version: result["projection"]["agora_start_version"]
                            .as_u64()
                            .unwrap_or(0),
                    },
                };
                let context_projection =
                    serde_json::from_value(result["projection"]["conscious_context"].clone()).ok();
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
                    projection: Some(projection),
                    context_projection,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::daemon_turn::test_support::DaemonTurnTestBuilder;

    fn context(thread: &str) -> PrincipalContext {
        PrincipalContext::new(
            fabric::PrincipalId(format!("test:{thread}")),
            fabric::LocalOsPrincipal {
                uid: nix::unistd::Uid::effective().as_raw(),
                gid: nix::unistd::Gid::effective().as_raw(),
            },
            fabric::ConnectionId::new(),
            fabric::ThreadId(thread.into()),
            fabric::WorkspacePolicy::from_resolved_roots(std::env::temp_dir(), Vec::new()).unwrap(),
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
        )
    }

    #[test]
    fn disabled_prompt_queue_preserves_direct_turn_admission() {
        assert_eq!(prompt_admission_mode(false), PromptAdmissionMode::Direct);
        assert_eq!(prompt_admission_mode(true), PromptAdmissionMode::Queued);
    }

    #[tokio::test]
    async fn execute_turn_success_runs_kernel_and_coordinator_lifecycle() {
        let harness = DaemonTurnTestBuilder::succeeding("mock answer")
            .build()
            .await;
        let response = harness
            .orchestrator
            .execute_turn(json!(7), "hello", context("daemon-success"))
            .await;

        assert_eq!(response["result"]["response"], "mock answer");
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
        assert!(harness
            .orchestrator
            .main_agent_process_id
            .lock()
            .await
            .is_some());
        let items = harness
            .store
            .load_items(&fabric::SessionId("daemon-success".into()), None)
            .await
            .unwrap();
        assert_eq!(
            items.len(),
            2,
            "coordinator persists user and terminal items"
        );
    }

    #[tokio::test]
    async fn execute_turn_error_settles_operation_and_returns_json_rpc_error() {
        let harness = DaemonTurnTestBuilder::failing("mock provider failed")
            .build()
            .await;
        let response = harness
            .orchestrator
            .execute_turn(json!(8), "hello", context("daemon-error"))
            .await;

        assert_eq!(response["error"]["code"], -32603);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mock provider failed"));
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
        assert!(harness
            .orchestrator
            .main_agent_process_id
            .lock()
            .await
            .is_some());
    }
}
