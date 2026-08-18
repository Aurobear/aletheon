//! TurnPipeline — shared turn orchestration for daemon and exec paths.
//!
//! Extracted from DaemonTurnOrchestrator::execute_turn so both the daemon
//! and CLI exec paths share the same Pre/Cognit/Post turn pipeline.

use crate::adapters::cognitive::daemon_session::{
    execute_cognitive_turn, operation_exit_reason, DaemonStreamingTurnContext,
};
#[cfg(test)]
use crate::adapters::cognitive::outcome::classify_runtime_turn_failure;
#[cfg(test)]
use crate::daemon::turn_event_projection::turn_event_to_client_event;
#[cfg(test)]
use crate::daemon::turn_event_projection::TerminalEventBuffer;
#[cfg(test)]
use ::contracts::ipc::TurnEventV1;
#[cfg(test)]
use ::contracts::ContentBlock;
use ::contracts::{Clock, PrincipalId, ProcessId, TurnRequest};
#[cfg(test)]
use adapters_inference::runtime_facts::bind_runtime_facts;
use agora::contract::AgoraService;
use application::turn::coordinator::{TurnExecution, TurnLifecycleHandle};
use application::turn::outcome::TurnPipelineRejection;
use cognit::CanonicalTurnEventSink;
#[cfg(test)]
use gateway::protocol::legacy_progress::ClientEvent;
use kernel::operation::OperationScope;
use kernel::KernelRuntime;
use runtime::event_projection::CanonicalEventBus;
use runtime::turn_pipeline_lifecycle::TurnPipelineEvent;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Typed outcome of a full turn pipeline run.
///
/// The pipeline owns every field of `TurnExecution` as a Rust type; this
/// replaces the JSON-RPC envelope it previously returned so the daemon engine
/// never reverse-indexes serialized turn/status JSON (R3).
pub enum TurnPipelineOutcome {
    Completed(Box<TurnExecution>),
    Rejected(TurnPipelineRejection),
}

/// Shared turn orchestration pipeline.
///
/// Bundles all state needed to execute a full cognitive turn:
/// pre-turn injection → SelfField review → memory composition → hooks →
/// tool setup → LLM selection → ReAct loop → event pumping →
/// post-turn settlement.
///
/// Constructed once per daemon instance and shared with DaemonTurnOrchestrator
/// via Arc fields.
#[allow(dead_code)]
pub struct TurnPipeline {
    pub notification: Option<Arc<dyn application::turn::service::TurnNotificationPort>>,
    pub clock: Arc<dyn Clock>,
    pub agora: Option<Arc<dyn AgoraService>>,
    pub kernel: Arc<KernelRuntime>,
    pub daemon_cancel_token: Option<CancellationToken>,
    pub context_assembler: Arc<application::turn::context::ContextAssembler>,
    pub canonical_sessions: Arc<::runtime::session_service::SessionService>,
    pub post_turn_projection: Arc<dyn application::turn::post_turn::PostTurnProjection>,
    pub hooks: Arc<dyn crate::adapters::hooks::TurnHookPort>,
    pub storm: Arc<dyn application::turn::ports::StormStatePort>,
    pub models: Arc<dyn application::turn::ports::ModelSelectionPort>,
    pub self_policy: Arc<dyn crate::adapters::conscious::self_policy::SelfPolicyPort>,
    pub approvals: Arc<dyn application::turn::ports::TurnApprovalPort>,
    pub capabilities: Arc<dyn crate::adapters::capability::GovernedTurnCapabilityPort>,
    pub sessions: Arc<dyn application::turn::ports::TurnSessionStatePort>,
    pub config: Arc<dyn application::turn::ports::TurnConfigPort>,
    pub observability: Arc<dyn application::turn::ports::TurnObservabilityPort>,
    pub cognitive_sessions: Arc<dyn ::cognit::harness::CognitiveSessionFactory>,
    pub conscious_core: Option<Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    pub session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub prompt_queue_enabled: bool,
    pub workspace_checkpoint: Arc<crate::host::workspace_checkpoint::WorkspaceCheckpointService>,
    pub lifecycle_adapter: Arc<crate::daemon::turn_lifecycle_adapter::TurnLifecycleAdapter>,
    pub role_workflow_factory: Option<Arc<::agora::cognitive_role_workflow::RoleWorkflowFactory>>,
    pub active_profile: Arc<dyn application::turn::ports::ActiveAgentProfilePort>,
    pub memory_gateway: Arc<::mnemosyne::MemoryGatewayService>,
    /// Previous stable prefix shape per canonical thread. Diagnostic only; it
    /// never gates inference or claims a provider-side cache hit.
    pub prefix_shape_trackers: Arc<Mutex<runtime::cache_shape::PrefixShapeTrackerStore>>,
}

pub struct TurnPipelineResources {
    pub notification: Option<Arc<dyn application::turn::service::TurnNotificationPort>>,
    pub clock: Arc<dyn Clock>,
    pub agora: Option<Arc<dyn AgoraService>>,
    pub kernel: Arc<KernelRuntime>,
    pub daemon_cancel: Option<CancellationToken>,
    pub context: Arc<application::turn::context::ContextAssembler>,
    pub canonical_sessions: Arc<::runtime::session_service::SessionService>,
    pub projection: Arc<dyn application::turn::post_turn::PostTurnProjection>,
    pub hooks: Arc<dyn crate::adapters::hooks::TurnHookPort>,
    pub storm: Arc<dyn application::turn::ports::StormStatePort>,
    pub models: Arc<dyn application::turn::ports::ModelSelectionPort>,
    pub self_policy: Arc<dyn crate::adapters::conscious::self_policy::SelfPolicyPort>,
    pub approvals: Arc<dyn application::turn::ports::TurnApprovalPort>,
    pub capabilities: Arc<dyn crate::adapters::capability::GovernedTurnCapabilityPort>,
    pub sessions: Arc<dyn application::turn::ports::TurnSessionStatePort>,
    pub config: Arc<dyn application::turn::ports::TurnConfigPort>,
    pub observability: Arc<dyn application::turn::ports::TurnObservabilityPort>,
    pub cognitive_sessions: Arc<dyn ::cognit::harness::CognitiveSessionFactory>,
    pub conscious_core: Option<Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    pub session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub prompt_queue_enabled: bool,
    pub workspace_checkpoint: Arc<crate::host::workspace_checkpoint::WorkspaceCheckpointService>,
    pub lifecycle: Arc<runtime::lifecycle_contributors::LifecycleRegistry>,
    pub lifecycle_enabled: bool,
    pub event_bus: Option<Arc<CanonicalEventBus>>,
    pub role_workflow_factory: Option<Arc<::agora::cognitive_role_workflow::RoleWorkflowFactory>>,
    pub active_profile: Arc<dyn application::turn::ports::ActiveAgentProfilePort>,
    pub memory_gateway: Arc<::mnemosyne::MemoryGatewayService>,
}

impl TurnPipeline {
    pub fn new(resources: TurnPipelineResources) -> Self {
        Self {
            notification: resources.notification,
            clock: resources.clock,
            agora: resources.agora,
            kernel: resources.kernel,
            daemon_cancel_token: resources.daemon_cancel,
            context_assembler: resources.context,
            canonical_sessions: resources.canonical_sessions,
            post_turn_projection: resources.projection,
            hooks: resources.hooks,
            storm: resources.storm,
            models: resources.models,
            self_policy: resources.self_policy,
            approvals: resources.approvals,
            capabilities: resources.capabilities,
            sessions: resources.sessions,
            config: resources.config,
            observability: resources.observability,
            cognitive_sessions: resources.cognitive_sessions,
            conscious_core: resources.conscious_core,
            session_input: resources.session_input,
            prompt_queue_enabled: resources.prompt_queue_enabled,
            workspace_checkpoint: resources.workspace_checkpoint,
            lifecycle_adapter: Arc::new(
                crate::daemon::turn_lifecycle_adapter::TurnLifecycleAdapter::new(
                    resources.lifecycle,
                    resources.lifecycle_enabled,
                    resources.event_bus,
                ),
            ),
            role_workflow_factory: resources.role_workflow_factory,
            active_profile: resources.active_profile,
            memory_gateway: resources.memory_gateway,
            prefix_shape_trackers: Arc::new(Mutex::new(Default::default())),
        }
    }

    /// Run the full Pre/Cognit/Post turn pipeline.
    ///
    /// Takes kernel-registered ids and the locally owned operation scope,
    /// then runs the entire turn: SelfField review, memory injection, hooks,
    /// tool setup, LLM selection, ReAct loop, event pumping, and post-turn
    /// settlement.
    pub async fn run(
        &self,
        message: String,
        turn_request: TurnRequest,
        main_pid: ProcessId,
        scope: &mut OperationScope,
        principal: PrincipalId,
        notification: Option<Arc<dyn application::turn::service::TurnNotificationPort>>,
    ) -> anyhow::Result<TurnPipelineOutcome> {
        let pipeline_started = std::time::Instant::now();
        let operation_id = scope.id;
        let scope_token = scope.token();
        // Resolve the authoritative runtime session before policy review. The
        // read is non-mutating; denied turns still never call `begin_user`.
        let requested_session_id = turn_request.context.thread_id.0.clone();
        // One owned profile snapshot governs budgeting, tool disclosure,
        // execution authority, cache identity, and post-turn compaction. A
        // concurrent profile switch becomes visible on the next turn only.
        let (current_session, config, turn_profile) = tokio::join!(
            self.sessions.current(&requested_session_id),
            self.config.config(),
            self.active_profile.snapshot()
        );
        let (session_id, current_turn_count) = current_session?;
        let turn_profile = turn_profile?;

        // TurnCoordinator already durably appended the user message, so the
        // cognitive adapter may bind clarification/root/role-graph state.
        let role_graph = crate::adapters::cognitive::role_graph::prepare_for_turn(
            self.agora.as_ref(),
            self.kernel.as_ref(),
            self.role_workflow_factory.as_ref(),
            &turn_request,
            &message,
            main_pid,
            scope_token.clone(),
            &turn_profile,
            &config,
        )
        .await?;

        // M6.43: checkpoint begin/finalize, terminal Fail/Cancel application and
        // abort dispatch ordering is owned by Application
        // (`application::turn::coordinator::run_turn_execution_envelope`). The
        // host builds the concrete checkpoint context and injects closures; it
        // no longer decides the execution-envelope sequence.
        let checkpoint_context = crate::host::workspace_checkpoint::CheckpointTurnContext {
            session_id: session_id.clone(),
            thread_id: turn_request.context.thread_id.0.clone(),
            turn_id: turn_request.context.turn_id.as_ref().map_or_else(
                || operation_id.0.to_string(),
                |turn_id| turn_id.0.to_string(),
            ),
            prompt_index: current_turn_count as u64 + 1,
            principal_id: principal.clone(),
            workspace: application::workspace_checkpoint::WorkspaceIdentity {
                canonical_path: turn_request.context.workspace.cwd().to_path_buf(),
                repo_fingerprint: None,
            },
            writable_roots: turn_request.context.workspace.writable_roots().to_vec(),
            created_at_ms: self.clock.wall_now().0,
        };
        let lifecycle_context = crate::daemon::turn_lifecycle_adapter::TurnLifecycleContext {
            principal_id: principal.clone(),
            thread_id: turn_request.context.thread_id.clone(),
            turn_id: turn_request.context.turn_id,
            session_id: session_id.clone(),
        };
        let lifecycle_principal = lifecycle_context.principal_id.clone();
        let lifecycle_turn = lifecycle_context.turn_id;
        let lifecycle_session = lifecycle_context.session_id.clone();
        let native_working_dir = turn_request.context.workspace.cwd().to_path_buf();
        let assistant_protocol_item = format!(
            "turn:{}:assistant",
            lifecycle_turn
                .unwrap_or(::contracts::TurnId(operation_id.0))
                .0
        );
        // Owned clones each envelope closure captures independently: the
        // execute closure moves the originals, so the begin/abort/finalize
        // closures must never borrow a value execution owns.
        let checkpoint_context_for_begin = checkpoint_context.clone();
        let workspace_checkpoint = self.workspace_checkpoint.clone();
        let lifecycle_adapter = self.lifecycle_adapter.clone();
        let abort_scope_token = scope_token.clone();
        let abort_lifecycle_context = lifecycle_context.clone();
        let cancel_scope_token = scope_token.clone();

        application::turn::coordinator::run_turn_execution_envelope(
            || {
                let checkpoint_context = checkpoint_context_for_begin.clone();
                let workspace_checkpoint = workspace_checkpoint.clone();
                async move { workspace_checkpoint.begin_turn(checkpoint_context).await }
            },
            move |lifecycle: TurnLifecycleHandle| async move {
        lifecycle.apply(TurnPipelineEvent::Admit)?;

        let native_turn_id = lifecycle_turn.unwrap_or(::contracts::TurnId(operation_id.0)).0.to_string();

        let preflight = crate::adapters::turn_preflight::HostTurnPreflight {
            lifecycle: self.lifecycle_adapter.clone(),
            lifecycle_context: lifecycle_context.clone(),
            cancel: scope_token.clone(),
            self_policy: self.self_policy.clone(),
            storm: self.storm.clone(),
            sessions: self.canonical_sessions.clone(),
            hooks: self.hooks.clone(),
        };
        let application::turn::context::PreparedTurnContext {
            request: context_request,
            context: prepared_context,
            model: llm,
            budget_costs: context_costs,
            sandbox: sandbox_requirement,
        } = match application::turn::context::prepare_pre_cognitive(
            &preflight,
            self.context_assembler.as_ref(),
            self.models.as_ref(),
            &turn_request,
            &message,
            &session_id,
            current_turn_count,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(application::turn::context::PrepareTurnError::Rejected(rejection)) => {
                return Ok(TurnPipelineOutcome::Rejected(rejection));
            }
            Err(application::turn::context::PrepareTurnError::Failed(error)) => return Err(error),
        };

        // Select exactly once before budget planning. The same provider instance
        // supplies authoritative capability facts, optional compaction, context
        // binding, and inference for this turn.
        let model_runtime_facts = llm.runtime_facts();
        let turn_id = turn_request
            .context
            .turn_id
            .ok_or_else(|| anyhow::anyhow!("context budget projection requires a canonical turn id"))?;
        let begin = self
            .sessions
            .begin_user(
                &requested_session_id,
                turn_id,
                &message,
                llm.clone(),
                turn_profile.clone(),
                context_costs,
            )
            .await?;
        let sess_id = begin.session_id;
        let turn_count = begin.turn_count;

        let existing_messages = crate::adapters::context_source::observe_and_resume_turn(
            self.memory_gateway.as_ref(),
            self.conscious_core.as_ref(),
            self.canonical_sessions.as_ref(),
            &lifecycle_principal,
            &lifecycle_session,
            &sess_id,
            &turn_request.context.thread_id.0,
            &native_turn_id,
            &native_working_dir,
            main_pid,
            operation_id,
            &message,
        )
        .await?;

        let evaluation_workspace = turn_request.context.workspace.clone();
        let evaluation_input = turn_request.input.clone();
        let evaluation_profile_name = turn_profile.profile_name.clone();
        let evaluation_effective_model_id = model_runtime_facts.effective_model_id.clone();
        let evaluation_model_display_name = model_runtime_facts.display_name.clone();
        let session_id_for_agora = sess_id.clone();
        let preparation_request = turn_request.clone();
        let preparation_config = config.clone();
        let preparation_scope_token = scope_token.clone();
        let preparation_lifecycle_context = lifecycle_context.clone();
        let cognition_scope_token = scope_token.clone();
        let cognition_lifecycle_context = lifecycle_context.clone();
        let cognition_lifecycle_session = lifecycle_session.clone();
        let cognition_assistant_protocol_item = assistant_protocol_item.clone();
        let cognition_llm = llm.clone();
        let cognition_session_id_for_agora = session_id_for_agora.clone();
        let lifecycle_for_cognition = lifecycle.clone();

        let (
            pumped,
            context_projection_receipt,
            agora_start_version,
            evaluation_diff_tracker,
            evaluation_capability_receipts,
            completed_inference_items,
            pre_cognitive_ms,
        ) = application::turn::coordinator::execute_capability_cognition_sequence(
            || {
                crate::adapters::cognitive::turn_preparation::prepare(
                    crate::adapters::cognitive::turn_preparation::PrepareCognitiveTurn {
                        agora: self.agora.as_ref(),
                        kernel: self.kernel.as_ref(),
                        conscious: self.conscious_core.as_ref(),
                        capabilities: self.capabilities.as_ref(),
                        context_assembler: self.context_assembler.as_ref(),
                        lifecycle: self.lifecycle_adapter.as_ref(),
                        lifecycle_context: &preparation_lifecycle_context,
                        prefix_trackers: self.prefix_shape_trackers.as_ref(),
                        session_input: self.session_input.clone(),
                        request: &preparation_request,
                        context_request: &context_request,
                        existing_messages: &existing_messages,
                        history_budget: begin.history_budget_tokens,
                        prepared_context,
                        model_facts: &model_runtime_facts,
                        profile: &turn_profile,
                        settings: &preparation_config,
                        principal: &principal,
                        process_id: main_pid,
                        operation_id,
                        session_id: &sess_id,
                        working_dir: &native_working_dir,
                        message: &message,
                        sandbox: sandbox_requirement,
                        turn_count,
                        rewrite_version: begin.rewrite_version,
                        cancel: preparation_scope_token.clone(),
                    },
                )
            },
            |prepared| async move {
                let crate::adapters::cognitive::turn_preparation::PreparedCognitiveTurn {
                    agora,
                    agora_version,
                    agora_start_version,
                    batch_planner,
                    mut turn_stream,
                    turn_sender,
                    tool_definitions: tool_defs,
                    tool_executor,
                    context_projection: context_projection_receipt,
                    request_messages,
                    prefix_shape_digest,
                    local_cache_miss_reason,
                    provider_miss_inference_allowed,
                    diff_tracker: evaluation_diff_tracker,
                    capability_receipts: evaluation_capability_receipts,
                    inference_items: completed_inference_items,
                } = prepared;
                lifecycle_for_cognition.apply(TurnPipelineEvent::ContextPrepared)?;
                let dasein_context = self.self_policy.dasein_context_provider();
                let agora_for_events = agora.clone();
                let capability_receipts = evaluation_capability_receipts.clone();
                let inference_items = completed_inference_items.clone();
                let execute_tool = move |tool_id: &str, name: &str, input: &serde_json::Value| {
                    let executor = tool_executor.clone();
                    let (tool_id, name, input) =
                        (tool_id.to_string(), name.to_string(), input.clone());
                    async move { executor.execute(tool_id, name, input).await }
                };
                let event_sink = CanonicalTurnEventSink::new(turn_sender);
                lifecycle_for_cognition.apply(TurnPipelineEvent::ToolLoopStarted)?;
                let pre_cognitive_ms = pipeline_started.elapsed().as_millis() as u64;

                let (react_result_tx, react_result_rx) = tokio::sync::oneshot::channel();
                let cognitive_sessions = self.cognitive_sessions.clone();
                let session_input = self.session_input.clone();
                let prompt_queue_enabled = self.prompt_queue_enabled;
                let react_cancel = cognition_scope_token.clone();
                let react_task_cancel = react_cancel.clone();
                let react_llm = cognition_llm.clone();
                scope.spawn("turn-react", async move {
                    let result = execute_cognitive_turn(
                        role_graph,
                        turn_request,
                        DaemonStreamingTurnContext {
                            config,
                            llm: react_llm,
                            tool_defs,
                            execute_tool,
                            event_sink,
                            request_messages,
                            dasein_context,
                            cancel_token: react_cancel,
                            sessions: cognitive_sessions,
                            batch_planner,
                            session_input,
                            prompt_queue_enabled,
                            capability_receipts,
                            inference_items,
                            prefix_shape_digest,
                            local_cache_miss_reason,
                            provider_miss_inference_allowed,
                        },
                    )
                    .await;
                    let reason = operation_exit_reason(&result, react_task_cancel.is_cancelled());
                    let _ = react_result_tx.send(result);
                    reason
                });

                let notify_tx: Option<Arc<dyn application::turn::service::TurnNotificationPort>> =
                    notification.or_else(|| self.notification.clone());
                let lifecycle_adapter = &self.lifecycle_adapter;
                let event_clock = self.clock.clone();
                let event_lifecycle_context = cognition_lifecycle_context.clone();
                let event_scope_token = cognition_scope_token.clone();
                let event_agora_session = cognition_session_id_for_agora.clone();
                let pumped = crate::daemon::turn_event_projection::pump_cognitive_turn(
                    react_result_rx,
                    &mut turn_stream,
                    self.approvals.as_ref(),
                    self.canonical_sessions.as_ref(),
                    &cognition_lifecycle_session,
                    lifecycle_turn.unwrap_or(::contracts::TurnId(operation_id.0)),
                    &cognition_assistant_protocol_item,
                    notify_tx.as_ref(),
                    agora_version,
                    move |version, tool| {
                        let agora = agora_for_events.clone();
                        let session_id = event_agora_session.clone();
                        let clock = event_clock.clone();
                        let lifecycle_context = event_lifecycle_context.clone();
                        let cancel = event_scope_token.clone();
                        async move {
                            let version = crate::adapters::conscious::turn_workspace::commit_tool_evidence(
                                agora.as_ref(),
                                &session_id,
                                main_pid,
                                version,
                                &tool.call_id,
                                &tool.name,
                                &tool.content,
                                tool.is_error,
                                clock.as_ref(),
                            ).await;
                            lifecycle_adapter.after_tool(&lifecycle_context, &tool, &cancel).await?;
                            Ok(version)
                        }
                    },
                )
                .await;
                let pumped = match pumped {
                    Ok(pumped) => pumped,
                    Err(error) => {
                        scope.cancel();
                        return Err(error);
                    }
                };
                Ok((
                    pumped,
                    context_projection_receipt,
                    agora_start_version,
                    evaluation_diff_tracker,
                    evaluation_capability_receipts,
                    completed_inference_items,
                    pre_cognitive_ms,
                ))
            },
        )
        .await?;
        let text = pumped.result;
        let mut evidence = pumped.evidence;
        let _final_agora_version = pumped.state;
        let terminal_events = pumped.terminal_events;

        // Terminal events are buffered while the ReAct task is running. They
        // must not be exposed to clients here: the coordinator still owns the
        // active-turn entry and durable terminal settlement after this
        // pipeline returns. The daemon orchestration boundary emits the
        // authoritative Error -> TurnDone sequence only after that settlement.
        let _buffered_terminal_events = terminal_events;
        let crate::adapters::cognitive::outcome::NormalizedCognitiveResult {
            mut result,
            execution_returned,
            runtime_faults,
        } = crate::adapters::cognitive::outcome::normalize(text);
        // A successfully returned runtime result can still be authoritatively
        // blocked (for example a Robot safety denial). Do not collapse "RPC
        // returned" into task success or let the rendered report advance the
        // Session as completed.
        let turn_succeeded = execution_returned && matches!(result.stop, ::contracts::TurnStop::Completed);
        // The Application evidence owner merges non-stream inference and
        // capability receipts, deduplicates them, and derives usage from the
        // resulting canonical item set.
        evidence.settle_result(
            &mut result,
            std::mem::take(&mut *completed_inference_items.lock().await),
            &evaluation_capability_receipts.lock().await,
        );
        let text = result.output.clone();
        let metrics = result.metrics.clone();
        info!(len = text.len(), "ReAct loop completed");
        let post_turn_started = std::time::Instant::now();

        lifecycle.apply(TurnPipelineEvent::ExecutionFinished)?;

        // -- Post-turn settlement --
        let post_effects = crate::adapters::turn_postflight::HostTurnPostEffects {
            self_policy: self.self_policy.clone(),
            memory_gateway: self.memory_gateway.clone(),
            principal: lifecycle_principal.clone(),
            session_id: lifecycle_session.clone(),
            turn_id: native_turn_id.clone(),
            working_dir: native_working_dir.clone(),
            assistant_item_id: assistant_protocol_item.clone(),
            lifecycle: self.lifecycle_adapter.clone(),
            lifecycle_context: lifecycle_context.clone(),
            cancel: scope_token.clone(),
        };
        let turn = evidence
            .settle_post_turn(
                &post_effects,
                self.observability.as_ref(),
                self.sessions.as_ref(),
                &requested_session_id,
                turn_count,
                &message,
                &result,
                turn_succeeded,
                llm,
                turn_profile,
                context_costs,
            )
            .await?;

        lifecycle.apply(TurnPipelineEvent::PostTurnSettled)?;
        lifecycle.apply(TurnPipelineEvent::ProjectionFinished)?;

        let evaluation_artifacts = application::evaluation::TurnEvaluationArtifacts::native(
            session_id_for_agora.clone(),
            evaluation_effective_model_id,
            evaluation_model_display_name,
            evaluation_workspace,
            evaluation_profile_name,
            evaluation_capability_receipts.lock().await.clone(),
            evaluation_diff_tracker.lock().await.snapshot(),
            runtime_faults,
            evidence.usage.projection_metrics(&metrics),
        );
        info!(
            pre_cognitive_ms,
            cognitive_ms = metrics.elapsed_ms,
            post_turn_ms = post_turn_started.elapsed().as_millis() as u64,
            total_elapsed_ms = pipeline_started.elapsed().as_millis() as u64,
            inference_rounds = metrics.iterations,
            provider_retries = metrics.provider_retries,
            tool_calls = metrics.tool_calls_made,
            "Turn latency breakdown"
        );
        Ok(TurnPipelineOutcome::Completed(Box::new(TurnExecution::completed(
            result,
            evidence.items,
            application::turn::outcome::CompletedTurnProjection {
                projector: self.post_turn_projection.clone(),
                session_id: session_id_for_agora,
                principal_id: principal,
                input: evaluation_input,
                turn,
                agora_start_version,
            },
            context_projection_receipt,
            evaluation_artifacts,
                ))))
            },
            || cancel_scope_token.is_cancelled(),
            || {
                let lifecycle_adapter = lifecycle_adapter.clone();
                let lifecycle_context = abort_lifecycle_context.clone();
                let scope_token = abort_scope_token.clone();
                async move {
                    lifecycle_adapter
                        .on_abort(&lifecycle_context, &scope_token)
                        .await
                }
            },
            |checkpoint_id, succeeded| {
                let workspace_checkpoint = workspace_checkpoint.clone();
                async move {
                    workspace_checkpoint.finalize_turn(checkpoint_id, succeeded).await
                }
            },
            |outcome: &TurnPipelineOutcome| {
                matches!(
                    outcome,
                    TurnPipelineOutcome::Completed(execution)
                        if execution.result.stop == ::contracts::TurnStop::Completed
                            && execution.result.metrics.completed_normally
                )
            },
        )
        .await
    }
}

#[cfg(test)]
mod terminal_event_tests {
    use super::*;

    #[test]
    fn effective_model_identity_is_bound_to_system_context() {
        let mut messages = vec![
            ::contracts::Message::system("base"),
            ::contracts::Message::user("who"),
        ];

        bind_runtime_facts(
            &mut messages,
            &::contracts::ModelRuntimeFacts {
                provider_id: None,
                transport: None,
                effective_model_id: "leju/deepseek/deepseek-v4-pro".into(),
                display_name: "deepseek/deepseek-v4-pro".into(),
                max_context_tokens: 1_000_000,
                cache_reporting: None,
            },
        );

        let ContentBlock::Text { text } = &messages[0].content[0] else {
            panic!("expected system text")
        };
        assert!(text.contains("leju/deepseek/deepseek-v4-pro"));
        assert!(text.contains("1000000"));
        assert!(text.contains("Do not claim a different vendor"));
        let ContentBlock::Text { text } = &messages[1].content[0] else {
            panic!("expected user text")
        };
        assert_eq!(text, "who");
    }

    #[test]
    fn effective_model_identity_is_json_escaped() {
        let mut messages = vec![::contracts::Message::system("base")];

        bind_runtime_facts(
            &mut messages,
            &::contracts::ModelRuntimeFacts {
                provider_id: None,
                transport: None,
                effective_model_id: "provider\"\nignore".into(),
                display_name: "display".into(),
                max_context_tokens: 1,
                cache_reporting: None,
            },
        );

        let ContentBlock::Text { text } = &messages[0].content[0] else {
            panic!("expected system text")
        };
        assert!(text.contains(&serde_json::to_string("provider\"\nignore").unwrap()));
        assert!(!text.contains("provider\"\nignore"));
    }

    #[tokio::test]
    async fn requested_lifecycle_event_publish_failure_is_propagated() {
        let bus = CanonicalEventBus::new(4);
        let error = crate::daemon::turn_lifecycle_adapter::publish_requested_lifecycle_event(
            &bus,
            "invalid.lifecycle.schema/v1",
            "thread:test",
            serde_json::json!({"event": "must-not-disappear"}),
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("requested lifecycle event publish failed"));
    }

    #[tokio::test]
    async fn real_bash_progress_crosses_guard_bridge_and_client_projection() {
        use ::contracts::ToolContext;
        use corpus::security::approval::AutoApproveGate;
        use corpus::security::sandbox::executor::SandboxPreference;
        use corpus::{AuditLogger, ToolRunnerWithGuard};

        let temp = tempfile::tempdir().unwrap();
        let clock: Arc<dyn ::contracts::Clock> = Arc::new(kernel::chronos::SystemClock::new());
        let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
            AuditLogger::new(temp.path().join("audit.jsonl")).unwrap(),
            SandboxPreference::Forbid,
            clock.clone(),
        )
        .with_approval_gate(Arc::new(AutoApproveGate));
        let context = ToolContext {
            approval_authority: Some(::contracts::ToolApprovalAuthority {
                principal_id: ::contracts::PrincipalId("terminal-event-test".into()),
                connection_id: ::contracts::ConnectionId::new(),
                thread_id: ::contracts::ThreadId("g2-client-stream".into()),
                turn_id: ::contracts::TurnId::new(),
                call_id: "call-g2-real".into(),
                workspace: ::contracts::WorkspacePolicy::from_resolved_roots(
                    temp.path().to_path_buf(),
                    vec![],
                )
                .unwrap(),
                granted_scope: ::contracts::CapabilityScope::default(),
                permission_mode: ::contracts::permission::HostPermissionMode::Safe,
            }),
            agent: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "g2-client-stream".into(),
            clock,
            turn_event_sender: None,
        };
        let (mut sink, event_rx) = ::contracts::tool_event_channel();

        let report = runner
            .execute_tool_streaming_report(
                &corpus::tools::bash_exec::BashExecTool,
                serde_json::json!({
                    "command": "printf 'alpha\\n'; sleep 0.02; printf 'beta\\n'",
                    "network_enabled": true
                }),
                &context,
                "g2-turn",
                &mut sink,
            )
            .await;
        assert!(
            report.result.is_ok(),
            "real BashExecTool must settle successfully: {:?}",
            report.result
        );
        drop(sink);

        let (mut turn_stream, turn_sender) = ::contracts::ipc::TurnEventStream::new();
        let outcome = runtime::tool_stream_bridge::bridge_tool_stream(
            event_rx,
            turn_sender,
            "bash_exec".into(),
            "call-g2-real".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(outcome.terminal.is_ok());
        assert!(outcome.progress_emitted > 0);

        let mut visible_text = String::new();
        while let Some(Ok(event)) = turn_stream.try_recv() {
            if let Some(ClientEvent::ToolProgress {
                call_id,
                tool,
                kind,
                payload,
            }) = turn_event_to_client_event(&event)
            {
                assert_eq!(call_id, "call-g2-real");
                assert_eq!(tool, "bash_exec");
                assert_eq!(kind, "text");
                visible_text.push_str(payload.as_str().expect("text progress payload"));
            }
        }
        assert!(visible_text.contains("alpha"));
        assert!(visible_text.contains("beta"));
    }

    #[test]
    fn failed_react_task_emits_exactly_error_then_turn_done() {
        let events =
            TerminalEventBuffer::default().into_client_events(Some("react task panicked".into()));

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ClientEvent::Error { message } if message == "react task panicked"
        ));
        assert!(matches!(&events[1], ClientEvent::TurnDone));
    }

    #[test]
    fn runtime_failure_classification_preserves_cancel_provider_and_crash_semantics() {
        let cancelled = anyhow::Error::new(cognit::CognitError::cancelled());
        let (stop, failure) = classify_runtime_turn_failure(&cancelled);
        assert_eq!(stop, ::contracts::TurnStop::Cancelled);
        assert!(failure.is_none());

        let transient = cognit::inference::InferenceFailure::transient("provider_unavailable");
        let (stop, failure) = classify_runtime_turn_failure(&transient);
        assert_eq!(stop, ::contracts::TurnStop::Failed);
        let failure = failure.unwrap();
        assert_eq!(
            failure.kind,
            ::contracts::TurnFailureKind::ProviderTransient
        );
        assert!(failure.retryable);

        let permanent = cognit::inference::InferenceFailure::terminal("provider_rejected_request");
        let (_, failure) = classify_runtime_turn_failure(&permanent);
        let failure = failure.unwrap();
        assert_eq!(
            failure.kind,
            ::contracts::TurnFailureKind::ProviderPermanent
        );
        assert!(!failure.retryable);

        let crash = anyhow::anyhow!("react task panicked");
        let (stop, failure) = classify_runtime_turn_failure(&crash);
        assert_eq!(stop, ::contracts::TurnStop::Failed);
        assert_eq!(failure.unwrap().kind, ::contracts::TurnFailureKind::Runtime);
    }

    #[test]
    fn reported_terminal_events_are_normalized_without_duplicates() {
        let mut buffered = TerminalEventBuffer::default();
        assert!(buffered.observe(&TurnEventV1::Error {
            message: "compaction failed".into(),
        }));
        assert!(buffered.observe(&TurnEventV1::TurnDone {
            result: Some("error: compaction failed".into()),
        }));

        let events = buffered.into_client_events(Some("fallback error".into()));

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ClientEvent::Error { message } if message == "compaction failed"
        ));
        assert!(matches!(&events[1], ClientEvent::TurnDone));
    }

    #[test]
    fn successful_react_task_emits_one_turn_done() {
        let mut buffered = TerminalEventBuffer::default();
        assert!(buffered.observe(&TurnEventV1::TurnDone {
            result: Some("ok".into()),
        }));

        let events = buffered.into_client_events(None);

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], ClientEvent::TurnDone));
    }

    #[test]
    fn tool_progress_projects_to_tui_wire_event() {
        let event = turn_event_to_client_event(&TurnEventV1::ToolProgress {
            name: "bash_exec".into(),
            call_id: "call-progress".into(),
            kind: "text".into(),
            payload: serde_json::json!("building"),
        });

        assert!(matches!(
            event,
            Some(ClientEvent::ToolProgress {
                call_id,
                tool,
                kind,
                payload,
            }) if call_id == "call-progress"
                && tool == "bash_exec"
                && kind == "text"
                && payload == serde_json::json!("building")
        ));
    }
}

#[cfg(test)]
#[path = "turn_pipeline_outcome_tests.rs"]
mod turn_pipeline_outcome_tests;
