//! execute_turn — the main orchestration entry point for daemon chat turns.
//!
//! This method handles kernel registration (process table, operation table) then
//! delegates the full Pre/Cognit/Post pipeline to `TurnPipeline::run()`.

use super::orchestrator::DaemonTurnOrchestrator;

use runtime::{PromptEnvelope, PromptKind};
use std::sync::Arc;

use ::contracts::{PrincipalContext, TurnRequest};
use gateway::protocol::legacy_progress::ClientEvent;
use runtime::turn_policy::TurnPolicy;
use serde_json::json;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptAdmissionMode {
    Direct,
    Queued,
}

/// Internal command-boundary result. Queue settlement consumes the typed stop
/// directly; it must never infer success from the presence of a JSON-RPC
/// `result` or `error` member.
struct TurnRpcExecution {
    response: serde_json::Value,
    stop: Option<::contracts::TurnStop>,
}

impl TurnRpcExecution {
    fn rejected(response: serde_json::Value) -> Self {
        Self {
            response,
            stop: None,
        }
    }

    fn settled(response: serde_json::Value, stop: ::contracts::TurnStop) -> Self {
        Self {
            response,
            stop: Some(stop),
        }
    }

    fn completed(&self) -> bool {
        self.stop == Some(::contracts::TurnStop::Completed)
    }
}

fn prompt_admission_mode(enabled: bool) -> PromptAdmissionMode {
    if enabled {
        PromptAdmissionMode::Queued
    } else {
        PromptAdmissionMode::Direct
    }
}

fn turn_failure_response(id: serde_json::Value, error: anyhow::Error) -> serde_json::Value {
    let (code, typed_code, retryable) =
        match error.downcast_ref::<application::turn::TurnEngineError>() {
            Some(engine_error) => {
                let code = match engine_error {
                    application::turn::TurnEngineError::Unavailable(_)
                    | application::turn::TurnEngineError::ProfileNotFound(_) => -32004,
                    application::turn::TurnEngineError::AdmissionRejected(_) => -32005,
                    application::turn::TurnEngineError::InvalidContext(_) => -32602,
                    application::turn::TurnEngineError::Internal(_) => -32603,
                };
                (code, engine_error.code(), engine_error.retryable())
            }
            None => (-32603, "turn_failed", false),
        };
    json!({"jsonrpc": "2.0", "id": id, "error": {
        "code": code,
        "message": error.to_string(),
        "data": {"code": typed_code, "retryable": retryable}
    }})
}

fn settled_turn_response(
    id: serde_json::Value,
    result: ::contracts::TurnResult,
) -> serde_json::Value {
    if result.stop != ::contracts::TurnStop::Failed {
        return json!({"jsonrpc": "2.0", "id": id, "result": {
            "response": result.output,
            "stop": result.stop,
            "failure": result.failure,
            "usage": result.usage,
            "metrics": result.metrics,
        }});
    }

    let failure = result.failure.unwrap_or(::contracts::TurnFailure {
        kind: ::contracts::TurnFailureKind::Unknown,
        message: result.output,
        retryable: false,
    });
    let (rpc_code, typed_code) = match failure.kind {
        ::contracts::TurnFailureKind::ProviderTransient => (-32004, "provider_unavailable"),
        ::contracts::TurnFailureKind::ProviderPermanent => (-32603, "provider_rejected_request"),
        ::contracts::TurnFailureKind::ContextOverflow => (-32006, "context_overflow"),
        ::contracts::TurnFailureKind::Tool => (-32603, "tool_failed"),
        ::contracts::TurnFailureKind::Policy => (-32003, "policy_denied"),
        ::contracts::TurnFailureKind::InvalidContext => (-32602, "turn_context_invalid"),
        ::contracts::TurnFailureKind::Persistence => (-32603, "turn_persistence_failed"),
        ::contracts::TurnFailureKind::Runtime => (-32603, "turn_runtime_failed"),
        ::contracts::TurnFailureKind::Unknown => (-32603, "turn_failed"),
    };
    json!({"jsonrpc": "2.0", "id": id, "error": {
        "code": rpc_code,
        "message": failure.message,
        "data": {
            "code": typed_code,
            "kind": failure.kind,
            "retryable": failure.retryable,
            "usage": result.usage,
            "metrics": result.metrics,
        }
    }})
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
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> serde_json::Value {
        self.execute_turn_with_context(
            id,
            message,
            context,
            requirements,
            task_kind,
            ::contracts::ExecutionTargetSelection::default(),
            notify,
            None,
        )
        .await
    }

    /// Execute a turn with an explicit target supplied by a typed trusted edge.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_turn_targeted(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> serde_json::Value {
        self.execute_turn_with_context(
            id,
            message,
            context,
            requirements,
            task_kind,
            execution_target,
            notify,
            None,
        )
        .await
    }

    /// Admit a turn and return the Runtime receipt while execution continues
    /// in a supervised detached task.
    pub async fn submit_turn_targeted(
        self: &std::sync::Arc<Self>,
        id: serde_json::Value,
        message: String,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> anyhow::Result<runtime::TurnId> {
        let (receipt_tx, receipt_rx) = tokio::sync::oneshot::channel();
        let orchestrator = self.clone();
        tokio::spawn(async move {
            let _ = orchestrator
                .execute_turn_with_context(
                    id,
                    &message,
                    context,
                    requirements,
                    task_kind,
                    execution_target,
                    notify,
                    Some(receipt_tx),
                )
                .await;
        });
        match receipt_rx.await {
            Ok(Ok(turn)) => Ok(turn),
            Ok(Err(error)) => anyhow::bail!(error),
            Err(_) => anyhow::bail!("turn rejected before Runtime admission"),
        }
    }

    /// Execute a channel turn under an identity established by the channel
    /// binding. The principal never comes from model-visible input.
    pub async fn execute_authenticated_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
    ) -> serde_json::Value {
        self.execute_turn_with_context(
            id,
            message,
            context,
            Vec::new(),
            None,
            ::contracts::ExecutionTargetSelection::default(),
            None,
            None,
        )
        .await
    }

    async fn execute_turn_with_context(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
        mut receipt: Option<tokio::sync::oneshot::Sender<Result<runtime::TurnId, String>>>,
    ) -> serde_json::Value {
        if let Err(error) = execution_target.validate() {
            return json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": -32602,
                "message": error,
                "data": {"code": "execution_target_invalid"}
            }});
        }
        if prompt_admission_mode(self.grok_hardening.prompt_queue) == PromptAdmissionMode::Direct {
            return self
                .execute_one_turn(
                    id,
                    message,
                    context,
                    requirements,
                    task_kind,
                    execution_target,
                    notify,
                    receipt.take(),
                )
                .await
                .response;
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
            .enqueue_with_target(
                principal.clone(),
                context.connection_id.clone(),
                thread.clone(),
                PromptKind::Prompt,
                message.to_owned(),
                idempotency_key,
                requirements,
                task_kind,
                execution_target,
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
            let prompt_notify = (prompt_id == queued.prompt_id)
                .then(|| notify.clone())
                .flatten();
            let execution = self
                .execute_queued_prompt(rpc_id, next, context.clone(), prompt_notify, receipt.take())
                .await;
            if execution.completed() {
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
                requested_result = Some(execution.response);
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
        notify: Option<tokio::sync::mpsc::Sender<String>>,
        receipt: Option<tokio::sync::oneshot::Sender<Result<runtime::TurnId, String>>>,
    ) -> TurnRpcExecution {
        context.connection_id = prompt.connection_id;
        context.thread_id = prompt.thread_id;
        self.execute_one_turn(
            id,
            &prompt.content,
            context,
            prompt.requirements,
            prompt.requested_task_kind,
            prompt.execution_target,
            notify,
            receipt,
        )
        .await
    }

    async fn execute_one_turn(
        &self,
        id: serde_json::Value,
        message: &str,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
        receipt: Option<tokio::sync::oneshot::Sender<Result<runtime::TurnId, String>>>,
    ) -> TurnRpcExecution {
        // -- Kernel: register main agent --
        let main_pid = match self
            .ensure_main_agent(&context.principal_id, &context.thread_id)
            .await
        {
            Ok(pid) => pid,
            Err(e) => {
                warn!(error = %e, "Failed to register main agent in process table");
                return TurnRpcExecution::rejected(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("Kernel error: {e}")}}),
                );
            }
        };

        // Resolve the active agent profile once per turn so model_policy,
        // prompt, budget, and approval are all drawn from the same source.
        let profile = match self.active_profile.snapshot().await {
            Ok(profile) => profile,
            Err(error) => {
                warn!(%error, "failed to resolve active turn profile");
                return TurnRpcExecution::rejected(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": error.to_string()}}),
                );
            }
        };
        let model_policy = profile.model_policy.clone();

        // The coordinator replaces this placeholder with the authoritative Turn id.
        let turn_request = TurnRequest {
            operation_id: ::contracts::OperationId::default(),
            process_id: main_pid,
            context,
            input: message.to_string(),
            execution_target: execution_target.clone(),
            model_policy,
            deadline: None,
            requirements,
            requested_task_kind: task_kind,
            evaluation_contract: None,
        };

        let _turn_token = self.begin_turn_token().await;
        let turn_engine = self.turn_engine.clone();
        let terminal_notify = notify.clone();
        #[cfg(test)]
        let test_runner = self.test_runner.clone();
        let policy = TurnPolicy::daemon();
        let coordinated = self
            .coordinator
            .submit_with_receipt(
                turn_request,
                &policy,
                receipt,
                move |request, cancel| async move {
                    #[cfg(test)]
                    if let Some(runner) = test_runner {
                        return runner(request, cancel).await;
                    }
                    let turn_engine = turn_engine
                        .ok_or_else(|| anyhow::anyhow!("daemon turn engine is not configured"))?;
                    let engine_result = turn_engine
                        .execute(
                            application::turn::TurnEngineRequest {
                                input: request.input.clone(),
                                model_policy: request.model_policy.clone(),
                                deadline: request.deadline,
                                requirements: request.requirements.clone(),
                                requested_task_kind: request.requested_task_kind,
                                execution_target: request.execution_target.clone(),
                            },
                            application::turn::TurnEngineContext {
                                principal_id: request.context.principal_id.clone(),
                                operation_id: request.operation_id,
                                process_id: request.process_id,
                                workspace: Arc::new(request.context.workspace.clone()),
                                profile: profile.clone(),
                                cancel_token: cancel,
                                principal_context: Some(request.context.clone()),
                                notification: notify.clone().map(|sender| {
                                    Arc::new(crate::daemon::turn_engine::MpscTurnNotificationPort(
                                        sender,
                                    ))
                                        as Arc<dyn application::turn::service::TurnNotificationPort>
                                }),
                            },
                        )
                        .await
                        .map_err(anyhow::Error::from)?;
                    engine_result.coordinator_execution.ok_or_else(|| {
                        anyhow::anyhow!(
                            "daemon turn engine omitted coordinator execution artifacts"
                        )
                    })
                },
            )
            .await;
        let terminal_error = match &coordinated {
            Ok(result) if result.stop == ::contracts::TurnStop::Failed => Some(
                result
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.clone())
                    .unwrap_or_else(|| result.output.clone()),
            ),
            Err(error) => Some(error.to_string()),
            _ => None,
        };
        self.emit_authoritative_terminal_events(
            terminal_notify,
            coordinated
                .as_ref()
                .ok()
                .map(|result| result.output.as_str())
                .filter(|output| !output.is_empty()),
            terminal_error.as_deref(),
        )
        .await;
        match coordinated {
            Ok(result) => {
                let stop = result.stop.clone();
                TurnRpcExecution::settled(settled_turn_response(id, result), stop)
            }
            Err(error) => TurnRpcExecution::rejected(turn_failure_response(id, error)),
        }
    }

    /// Notify the client only after `TurnCoordinator::submit_with` has removed
    /// the active entry and settled the kernel operation. A Cognit TurnDone is
    /// a pipeline-local result, not authoritative permission to start another
    /// turn on the same thread.
    async fn emit_authoritative_terminal_events(
        &self,
        sender: Option<tokio::sync::mpsc::Sender<String>>,
        output: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(sender) = sender.or_else(|| self.notify_tx.try_lock().ok()?.clone()) else {
            return;
        };
        let mut events =
            Vec::with_capacity(1 + usize::from(output.is_some()) + usize::from(error.is_some()));
        if let Some(text) = output {
            events.push(ClientEvent::TextSnapshot {
                text: text.to_owned(),
            });
        }
        if let Some(error) = error {
            events.push(ClientEvent::Error {
                message: error.to_owned(),
            });
        }
        events.push(ClientEvent::TurnDone);
        for event in events {
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "event",
                "params": event,
            });
            let Ok(payload) = serde_json::to_string(&notification) else {
                warn!("unable to serialize authoritative terminal turn event");
                continue;
            };
            if sender.send(payload).await.is_err() {
                warn!("event sink closed before authoritative terminal turn event");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::turn::test_support::DaemonTurnTestBuilder;

    fn context(thread: &str) -> PrincipalContext {
        PrincipalContext::new(
            ::contracts::PrincipalId(format!("test:{thread}")),
            ::contracts::LocalOsPrincipal {
                uid: nix::unistd::Uid::effective().as_raw(),
                gid: nix::unistd::Gid::effective().as_raw(),
            },
            ::contracts::ConnectionId::new(),
            ::contracts::ThreadId(thread.into()),
            ::contracts::WorkspacePolicy::from_resolved_roots(std::env::temp_dir(), Vec::new())
                .unwrap(),
            ::contracts::PermissionProfileId::workspace_write(),
            ::contracts::ApprovalPolicy::OnRequest,
        )
    }

    #[test]
    fn disabled_prompt_queue_preserves_direct_turn_admission() {
        assert_eq!(prompt_admission_mode(false), PromptAdmissionMode::Direct);
        assert_eq!(prompt_admission_mode(true), PromptAdmissionMode::Queued);
    }

    #[test]
    fn queued_prompt_completion_uses_typed_stop_not_json_shape() {
        for (stop, completed) in [
            (::contracts::TurnStop::Completed, true),
            (::contracts::TurnStop::Blocked, false),
            (::contracts::TurnStop::Cancelled, false),
            (::contracts::TurnStop::Failed, false),
        ] {
            let execution = TurnRpcExecution::settled(
                json!({"jsonrpc": "2.0", "result": {"legacy_success_shape": true}}),
                stop,
            );
            assert_eq!(execution.completed(), completed);
        }

        let rejected = TurnRpcExecution::rejected(json!({
            "jsonrpc": "2.0",
            "error": {"code": -32603}
        }));
        assert!(!rejected.completed());
    }

    #[test]
    fn unavailable_turn_error_keeps_typed_json_rpc_contract() {
        let error = anyhow::Error::new(application::turn::TurnEngineError::Unavailable(
            "execution_target_unavailable: robot capability is not configured".into(),
        ));
        let response = turn_failure_response(json!(72), error);
        assert_eq!(response["error"]["code"], -32004);
        assert_eq!(
            response["error"]["data"]["code"],
            "execution_target_unavailable"
        );
        assert_eq!(response["error"]["data"]["retryable"], true);
    }

    #[test]
    fn turn_engine_error_variants_preserve_machine_code_and_retryability() {
        use application::turn::TurnEngineError;

        for (error, rpc_code, typed_code, retryable) in [
            (
                TurnEngineError::ProfileNotFound("missing".into()),
                -32004,
                "turn_profile_not_found",
                false,
            ),
            (
                TurnEngineError::AdmissionRejected("busy".into()),
                -32005,
                "turn_admission_rejected",
                false,
            ),
            (
                TurnEngineError::InvalidContext("missing identity".into()),
                -32602,
                "turn_context_invalid",
                false,
            ),
            (
                TurnEngineError::Internal(anyhow::anyhow!("runtime crash")),
                -32603,
                "turn_runtime_failed",
                false,
            ),
        ] {
            let response = turn_failure_response(json!(91), anyhow::Error::new(error));
            assert_eq!(response["error"]["code"], rpc_code);
            assert_eq!(response["error"]["data"]["code"], typed_code);
            assert_eq!(response["error"]["data"]["retryable"], retryable);
        }
    }

    #[test]
    fn settled_failed_and_blocked_turns_keep_distinct_typed_contracts() {
        let failed = settled_turn_response(
            json!(92),
            ::contracts::TurnResult {
                output: "provider unavailable".into(),
                stop: ::contracts::TurnStop::Failed,
                failure: Some(::contracts::TurnFailure {
                    kind: ::contracts::TurnFailureKind::ProviderTransient,
                    message: "provider unavailable".into(),
                    retryable: true,
                }),
                usage: ::contracts::InferenceUsage::reported(10, 2, None, None, None),
                metrics: ::contracts::TurnMetrics::default(),
            },
        );
        assert_eq!(failed["error"]["data"]["code"], "provider_unavailable");
        assert_eq!(failed["error"]["data"]["retryable"], true);
        assert_eq!(failed["error"]["data"]["usage"]["total_input_tokens"], 10);

        let blocked = settled_turn_response(
            json!(93),
            ::contracts::TurnResult {
                output: "policy denied".into(),
                stop: ::contracts::TurnStop::Blocked,
                failure: None,
                usage: ::contracts::InferenceUsage::default(),
                metrics: ::contracts::TurnMetrics {
                    tool_errors: 1,
                    ..Default::default()
                },
            },
        );
        assert_eq!(blocked["result"]["stop"], "Blocked");
        assert!(blocked.get("error").is_none());
    }

    #[tokio::test]
    async fn invalid_robot_target_is_rejected_as_typed_input_error() {
        let harness = DaemonTurnTestBuilder::succeeding("must not run")
            .build()
            .await;
        let response = harness
            .orchestrator
            .execute_turn_targeted(
                json!(73),
                "move",
                context("invalid-target"),
                Vec::new(),
                None,
                ::contracts::ExecutionTargetSelection {
                    target: ::contracts::ExecutionTarget::Robot {
                        device_id: ::contracts::types::embodiment::DeviceId("robot-1".into()),
                        environment:
                            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
                    },
                    source: ::contracts::ExecutionTargetSource::Default,
                },
                None,
            )
            .await;
        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(
            response["error"]["data"]["code"],
            "execution_target_invalid"
        );
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
    }

    #[tokio::test]
    async fn execute_turn_success_runs_kernel_and_coordinator_lifecycle() {
        let harness = DaemonTurnTestBuilder::succeeding("mock answer")
            .build()
            .await;
        let response = harness
            .orchestrator
            .execute_turn(
                json!(7),
                "hello",
                context("daemon-success"),
                Vec::new(),
                None,
                None,
            )
            .await;

        assert_eq!(response["result"]["response"], "mock answer");
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
        assert!(harness
            .orchestrator
            .main_agent_process_ids
            .lock()
            .await
            .contains_key("test:daemon-success\0daemon-success"));
        let items = harness
            .store
            .load_items(&::contracts::SessionId("daemon-success".into()), None)
            .await
            .unwrap();
        assert_eq!(
            items.len(),
            2,
            "coordinator persists user and terminal items"
        );
    }

    #[tokio::test]
    async fn async_submit_returns_runtime_receipt_before_terminal_settlement() {
        let runner = Arc::new(move |_request: TurnRequest, _cancel| {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                Ok(application::turn::coordinator::TurnExecution {
                    result: ::contracts::TurnResult {
                        output: "detached answer".into(),
                        stop: ::contracts::TurnStop::Completed,
                        failure: None,
                        usage: Default::default(),
                        metrics: ::contracts::TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: Vec::new(),
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            }) as futures::future::BoxFuture<'static, _>
        });
        let harness = DaemonTurnTestBuilder::new(runner).build().await;
        let coordinator = harness.coordinator.clone();
        let orchestrator = Arc::new(harness.orchestrator);
        let started = std::time::Instant::now();
        let turn = orchestrator
            .submit_turn_targeted(
                json!(74),
                "detached".into(),
                context("detached-admission"),
                Vec::new(),
                None,
                ::contracts::ExecutionTargetSelection::default(),
                None,
            )
            .await
            .unwrap();
        assert!(!turn.0.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_millis(90));
        assert_eq!(coordinator.active_turn_count().await, 1);
        for _ in 0..20 {
            if coordinator.active_turn_count().await == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(coordinator.active_turn_count().await, 0);
    }

    #[tokio::test]
    async fn terminal_client_event_is_emitted_after_active_turn_release() {
        let harness = DaemonTurnTestBuilder::succeeding("mock answer")
            .build()
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let (unrelated_tx, mut unrelated_rx) = tokio::sync::mpsc::channel(4);
        harness.orchestrator.set_notify_sender(unrelated_tx).await;
        let response = harness
            .orchestrator
            .execute_turn(
                json!(71),
                "hello",
                context("daemon-terminal-order"),
                Vec::new(),
                None,
                Some(tx),
            )
            .await;

        assert_eq!(response["result"]["response"], "mock answer");
        let snapshot = rx.recv().await.expect("authoritative text snapshot");
        assert!(snapshot.contains("text_snapshot"));
        assert!(snapshot.contains("mock answer"));
        let terminal = rx.recv().await.expect("authoritative terminal event");
        assert!(terminal.contains("turn_done"));
        assert!(
            unrelated_rx.try_recv().is_err(),
            "another connection stole Turn events"
        );
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
    }

    #[tokio::test]
    async fn execute_turn_error_settles_operation_and_returns_json_rpc_error() {
        let harness = DaemonTurnTestBuilder::failing("mock provider failed")
            .build()
            .await;
        let response = harness
            .orchestrator
            .execute_turn(
                json!(8),
                "hello",
                context("daemon-error"),
                Vec::new(),
                None,
                None,
            )
            .await;

        assert_eq!(response["error"]["code"], -32603);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mock provider failed"));
        assert_eq!(harness.coordinator.active_turn_count().await, 0);
        assert!(harness
            .orchestrator
            .main_agent_process_ids
            .lock()
            .await
            .contains_key("test:daemon-error\0daemon-error"));
    }

    #[tokio::test]
    async fn typed_task_kind_and_target_survive_direct_and_queued_admission() {
        for (queued, thread) in [(false, "task-kind-direct"), (true, "task-kind-queued")] {
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed_by_runner = observed.clone();
            let runner = Arc::new(move |request: TurnRequest, _cancel| {
                observed_by_runner.lock().unwrap().push((
                    request.requested_task_kind,
                    request.execution_target.clone(),
                ));
                Box::pin(async move {
                    Ok(application::turn::coordinator::TurnExecution {
                        result: ::contracts::TurnResult {
                            output: "typed turn".into(),
                            stop: ::contracts::TurnStop::Completed,
                            failure: None,
                            usage: Default::default(),
                            metrics: ::contracts::TurnMetrics {
                                completed_normally: true,
                                ..Default::default()
                            },
                        },
                        items: Vec::new(),
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: Default::default(),
                    })
                }) as futures::future::BoxFuture<'static, _>
            });
            let harness = DaemonTurnTestBuilder::new(runner)
                .with_prompt_queue(queued)
                .build()
                .await;

            let response = harness
                .orchestrator
                .execute_turn_targeted(
                    json!(thread),
                    "explicit coding turn",
                    context(thread),
                    Vec::new(),
                    Some(::contracts::TaskKind::Coding),
                    ::contracts::ExecutionTargetSelection::robot(
                        "robot-1",
                        ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
                        ::contracts::ExecutionTargetSource::TrustedClient,
                    )
                    .unwrap(),
                    None,
                )
                .await;

            assert_eq!(response["result"]["response"], "typed turn");
            assert_eq!(
                observed.lock().unwrap().as_slice(),
                &[(
                    Some(::contracts::TaskKind::Coding),
                    ::contracts::ExecutionTargetSelection::robot(
                        "robot-1",
                        ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
                        ::contracts::ExecutionTargetSource::TrustedClient,
                    )
                    .unwrap(),
                )],
                "queued={queued}"
            );
        }
    }
}
