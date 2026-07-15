//! TurnPipeline — shared turn orchestration for daemon and exec paths.
//!
//! Extracted from DaemonTurnOrchestrator::execute_turn so both the daemon
//! and CLI exec paths share the same Pre/Cognit/Post turn pipeline.

use crate::core::session_gateway::SessionGateway;
use crate::service::daemon_react::{submit_streaming_daemon_turn, DaemonStreamingTurnContext};
use crate::service::daemon_turn::helpers::bounded_text_history;
use crate::service::governed_capability::CapabilityExecutionContext;
use aletheon_kernel::operation::OperationScope;
use aletheon_kernel::KernelRuntime;
use cognit::harness::event_sink::{ChannelEventSink, Event};
use cognit::harness::linear::TurnMetrics;
use fabric::events::ui_event::ClientEvent;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::include::agora::{AgoraOperation, WorkspaceCommitPermit};
use fabric::ipc::{StreamConfig, TurnEventStream, TurnEventV1};
use fabric::{
    AgoraOps, AgoraSpaceId, AgoraVersion, CapabilityCall, Clock, ContentBlock, ContextBinding,
    Intent, IntentSource, OperationId, PrincipalId, ProcessId, Role, SandboxRequirement, SessionId,
    SpaceId, TurnRequest,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

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
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) agora: Option<Arc<dyn AgoraOps>>,
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) current_scope: Arc<Mutex<Option<OperationScope>>>,
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
    pub(crate) context_assembler: Arc<crate::service::context_assembler::ContextAssembler>,
    pub(crate) canonical_sessions: Arc<crate::service::session_service::SessionService>,
    pub(crate) post_turn_projection:
        Arc<dyn crate::service::post_turn_projection::PostTurnProjection>,
    pub(crate) runtime_ports: Arc<crate::service::turn_runtime_ports::TurnRuntimePorts>,
}

pub(crate) struct TurnPipelineResources {
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) agora: Option<Arc<dyn AgoraOps>>,
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) current_scope: Arc<Mutex<Option<OperationScope>>>,
    pub(crate) daemon_cancel: Option<CancellationToken>,
    pub(crate) context: Arc<crate::service::context_assembler::ContextAssembler>,
    pub(crate) canonical_sessions: Arc<crate::service::session_service::SessionService>,
    pub(crate) projection: Arc<dyn crate::service::post_turn_projection::PostTurnProjection>,
    pub(crate) runtime: Arc<crate::service::turn_runtime_ports::TurnRuntimePorts>,
}

impl TurnPipeline {
    pub(crate) fn new(resources: TurnPipelineResources) -> Self {
        Self {
            session_gateway: resources.session_gateway,
            notify_tx: resources.notify,
            clock: resources.clock,
            agora: resources.agora,
            kernel: resources.kernel,
            current_scope: resources.current_scope,
            daemon_cancel_token: resources.daemon_cancel,
            context_assembler: resources.context,
            canonical_sessions: resources.canonical_sessions,
            post_turn_projection: resources.projection,
            runtime_ports: resources.runtime,
        }
    }

    /// Run the full Pre/Cognit/Post turn pipeline.
    ///
    /// Takes kernel-registered ids and a scope token from the orchestrator,
    /// then runs the entire turn: SelfField review, memory injection, hooks,
    /// tool setup, LLM selection, ReAct loop, event pumping, and post-turn
    /// settlement.
    pub async fn run(
        &self,
        id: serde_json::Value,
        message: String,
        turn_request: TurnRequest,
        operation_id: OperationId,
        main_pid: ProcessId,
        scope_token: CancellationToken,
        principal: PrincipalId,
    ) -> anyhow::Result<serde_json::Value> {
        // -- SelfField review --
        let intent = Intent {
            action: "chat".to_string(),
            parameters: serde_json::json!({"message": message}),
            source: IntentSource::User,
            description: {
                let end = message
                    .char_indices()
                    .nth(80)
                    .map(|(i, _)| i)
                    .unwrap_or(message.len());
                format!("User chat message: {}", &message[..end])
            },
        };
        let sf_ctx =
            fabric::Context::new(&turn_request.session_id, turn_request.working_dir.clone());
        let verdict = self
            .runtime_ports
            .self_policy
            .review(&intent, &sf_ctx)
            .await;

        // Sandbox requirement from SelfField verdict — passed to tool admission.
        let mut sandbox_requirement = SandboxRequirement::NotRequired;

        match verdict {
            Ok(fabric::Verdict::Deny { ref reason }) => {
                warn!(reason = %reason, "SelfField denied chat intent");
                self.runtime_ports
                    .self_policy
                    .narrate("chat_denied", reason)
                    .await;
                return Ok(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32010, "message": format!("Intent denied by SelfField: {}", reason)}}),
                );
            }
            Ok(fabric::Verdict::SandboxFirst { ref reason }) => {
                warn!(reason = %reason, "SelfField requires sandbox; tools will be gated through admission");
                self.runtime_ports
                    .self_policy
                    .narrate("chat_sandbox_required", reason)
                    .await;
                sandbox_requirement = SandboxRequirement::Required;
            }
            Err(ref e) => {
                warn!(error = %e, "SelfField review error — denying turn (fail-closed)");
                return Ok(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32010, "message": format!("SelfField review failed (fail-closed): {}", e)}}),
                );
            }
            _ => {}
        }

        // -- Context source preparation --
        self.runtime_ports.storm.reset().await;
        let mut effective_message = String::new();

        // -- Configured pre_turn hook scripts --
        let hook_session_id = self.runtime_ports.sessions.current().await?.0;
        effective_message.push_str(
            &self
                .runtime_ports
                .hooks
                .run_pre_turn_script(&message, &hook_session_id)
                .await,
        );

        effective_message.push_str(&message);

        // -- PreTurn hooks --
        {
            let (sess_id, turn_count) = self.runtime_ports.sessions.current().await?;
            let ctx = HookContext {
                point: HookPoint::PreTurn,
                session_id: sess_id,
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.clone()),
                metadata: HashMap::new(),
            };
            match self.runtime_ports.hooks.execute(ctx).await {
                HookResult::Block { reason } => {
                    warn!(reason = %reason, "PreTurn hook blocked");
                    return Ok(
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32015, "message": format!("Blocked by hook: {}", reason)}}),
                    );
                }
                HookResult::Inject(text) => {
                    effective_message.push_str(&text);
                    effective_message.push('\n');
                }
                _ => {}
            }
        }

        let (sess_id, turn_count) = self.runtime_ports.sessions.begin_user(&message).await?;

        // Canonical Session/Turn/Item history is the only model replay source.
        let existing_messages = {
            let mut full_history = self
                .canonical_sessions
                .resume(&fabric::SessionId(turn_request.session_id.clone()))
                .await?
                .messages;
            if full_history.last().is_some_and(|last| {
                last.role == Role::User
                    && last.content.iter().any(
                        |block| matches!(block, ContentBlock::Text { text } if text == &message),
                    )
            }) {
                full_history.pop();
            }
            bounded_text_history(&full_history)
        };

        let mut context_request = turn_request.clone();
        context_request.input = effective_message;
        let request_messages = self
            .context_assembler
            .assemble(&context_request, &existing_messages)
            .await?
            .messages;

        // LLM selection
        let llm = self.runtime_ports.models.select(&message);

        // -- Governed capability setup --
        let working_dir = turn_request.working_dir.clone();

        // Context Space seed — user turn input is private overlay data, not
        // shared Agora fact. Shared visibility requires an explicit proposal.
        let agora = self.agora.clone();
        let mut agora_version = if let Some(ref agora) = agora {
            agora.version(&sess_id).await.unwrap_or(0)
        } else {
            tracing::warn!(target: "agora", "DomainPorts.agora is not configured; shared evidence commits disabled for this turn");
            0
        };
        let agora_start_version = agora_version;
        // Phase 2a: reuse the main agent's long-lived process space (one per
        // session, not per turn). Bindings are upserted so the Agora version is
        // refreshed in place rather than accumulating. Space is released on
        // process exit (see orchestrator::exit_process).
        let agent_space = match self.kernel.inspect_process(main_pid).await {
            Ok(snap) => snap.space,
            Err(e) => {
                tracing::warn!(target: "space", error = %e, "inspect(main_pid) failed; using ephemeral space for this turn");
                SpaceId::new()
            }
        };
        self.kernel.upsert_space_binding(
            agent_space,
            ContextBinding::Session(SessionId(sess_id.clone())),
        );
        self.kernel.upsert_space_binding(
            agent_space,
            ContextBinding::Agora(AgoraSpaceId(sess_id.clone()), AgoraVersion(agora_version)),
        );
        if let Err(e) =
            self.kernel
                .set_space_overlay(agent_space, "turn_input", serde_json::json!(message))
        {
            tracing::warn!(target: "space", error = %e, "failed to store turn input overlay");
        }

        let dasein_context = self.runtime_ports.self_policy.dasein_context_provider();
        let session_id_for_agora = sess_id.clone();
        let agora_for_events = agora.clone();
        let clock_for_agora = self.clock.clone();

        let prepared = self
            .runtime_ports
            .capabilities
            .prepare(CapabilityExecutionContext {
                process_id: main_pid,
                operation_id,
                principal,
                session_id: sess_id.clone(),
                working_dir,
                sandbox: sandbox_requirement,
                cancel: scope_token.clone(),
                turn_count,
            })
            .await?;
        let tool_defs = prepared.definitions;
        let capability = prepared.invoker;

        let execute_tool = move |tool_id: &str, name: &str, input: &serde_json::Value| {
            let capability = capability.clone();
            let (tid, n, inp) = (tool_id.to_string(), name.to_string(), input.clone());
            async move {
                let result = capability
                    .invoke(CapabilityCall {
                        operation_id,
                        process_id: main_pid,
                        name: n,
                        input: inp,
                        call_id: tid,
                        deadline: None,
                    })
                    .await;
                (result.output, result.is_error)
            }
        };

        // Event channel — kept for ReAct loop (ChannelEventSink)
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(64);
        let event_sink = ChannelEventSink::new(event_tx.clone());
        drop(event_tx); // Only ChannelEventSink holds a sender; drops when react_task completes

        // Turn event stream — EnvelopeV2-typed channel for turn orchestration
        let (mut turn_stream, turn_sender) = TurnEventStream::new(StreamConfig::turn_events(64));

        // Bridge task: convert Event → TurnEventV1 → EnvelopeV2
        let _bridge_task = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let turn_event = convert_event_to_turn_event(event);
                if turn_sender.send(&turn_event).is_err() {
                    break;
                }
            }
        });

        let config = self.runtime_ports.config.config().await;
        let goal_message = message.to_string();
        let goal_message_for_gw = goal_message.clone();

        // Spawn ReAct loop
        let mut react_task = tokio::spawn(submit_streaming_daemon_turn(
            turn_request,
            DaemonStreamingTurnContext {
                config,
                llm: llm.clone(),
                tool_defs,
                execute_tool,
                event_sink,
                request_messages,
                dasein_context,
                cancel_token: scope_token,
                clock: self.clock.clone(),
            },
        ));

        // -- Event + approval pumping loop --
        let notify_tx = self.notify_tx.clone();

        let mut tool_calls_for_session: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut tool_results_for_session: Vec<(String, String, bool)> = Vec::new();
        let mut canonical_items: Vec<fabric::ItemPayload> = Vec::new();
        let mut acc_tokens_in: u64 = 0;
        let mut acc_tokens_out: u64 = 0;

        let text = loop {
            tokio::select! {
                result = &mut react_task => {
                    break result.unwrap_or_else(|e| Err(anyhow::anyhow!("react task panicked: {e}")));
                }
                event_result = turn_stream.recv() => {
                    let event = match event_result {
                        Ok(ev) => ev,
                        Err(rejection) => {
                            warn!(schema = %rejection.actual, "Schema mismatch in turn event stream, skipping event");
                            continue;
                        }
                    };
                    match &event {
                        TurnEventV1::ToolCallStart { name, call_id } => {
                            tool_calls_for_session.push((call_id.clone(), name.clone(), serde_json::Value::Null));
                        }
                        TurnEventV1::ToolCallComplete { call_id, name: _, args } => {
                            if let Some(tc) = tool_calls_for_session.iter_mut().find(|(id, _, _)| id == call_id) {
                                tc.2 = args.clone();
                                canonical_items.push(fabric::ItemPayload::ToolCall {
                                    call_id: call_id.clone(), name: tc.1.clone(), input: args.clone(),
                                });
                            }
                        }
                        TurnEventV1::ToolResult { name, call_id, content, is_error, .. } => {
                            tool_results_for_session.push((call_id.clone(), content.clone(), *is_error));
                            canonical_items.push(fabric::ItemPayload::ToolResult {
                                call_id: call_id.clone(), content: content.clone(), is_error: *is_error,
                                permit_id: None, audit_id: None,
                            });
                            let evidence = fabric::Evidence::from_tool_result(
                                call_id.clone(), name.clone(), content.clone(), *is_error,
                            );
                            if let Some(ref agora) = agora_for_events {
                                match agora.propose(
                                    &session_id_for_agora,
                                    agora_version,
                                    AgoraOperation::AcceptEvidence { evidence },
                                    main_pid,
                                ).await {
                                    Ok(prop) => {
                                        let permit = WorkspaceCommitPermit::issue_for(
                                            &prop,
                                            clock_for_agora.wall_now().0.saturating_add(30_000),
                                        );
                                        let result = match permit {
                                            Ok(permit) => agora.commit_with_permit(
                                                &session_id_for_agora,
                                                prop.id,
                                                permit,
                                            ).await,
                                            Err(error) => Err(error.to_string()),
                                        };
                                        if let Err(e) = result {
                                            tracing::warn!(target: "agora", error = %e, "agora commit (evidence) failed");
                                        } else {
                                            agora_version += 1;
                                        }
                                    }
                                    Err(e) => tracing::warn!(target: "agora", error = %e, "agora propose (evidence) failed"),
                                }
                            } else {
                                tracing::warn!(
                                    target: "agora",
                                    "DomainPorts.agora missing; skipping shared evidence commit"
                                );
                            }
                        }
                        TurnEventV1::Usage { tokens_in, tokens_out, .. } => {
                            acc_tokens_in += *tokens_in as u64;
                            acc_tokens_out += *tokens_out as u64;
                        }
                        _ => {}
                    }
                    // Forward event to TUI client
                    {
                        let guard = notify_tx.lock().await;
                        if let Some(ref tx) = *guard {
                            if let Some(client_event) = turn_event_to_client_event(&event) {
                                if let Ok(json_str) = event_to_json(&client_event) {
                                    if tx.send(json_str).await.is_err() {
                                        debug!("Event sink closed, dropping event");
                                    }
                                }
                            }
                        }
                    }
                }
                Some(pending) = self.runtime_ports.approvals.next() => {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "approval_request",
                        "params": {
                            "approval_id": pending.approval_id,
                            "tool": pending.tool,
                            "action_summary": pending.action_summary,
                            "risk_level": pending.risk_level,
                            "detail": pending.detail,
                        }
                    });
                    {
                        let guard = notify_tx.lock().await;
                        if let Some(ref tx) = *guard {
                            if tx.send(notification.to_string()).await.is_err() {
                                warn!("Failed to send approval_request notification — client disconnected?");
                            }
                        } else {
                            warn!("No notify_tx configured — approval request will timeout (fail-safe deny)");
                        }
                    }
                }
            }
        };

        // Drain remaining events from turn stream
        // Yield once to allow the bridge task to forward inflight events
        tokio::task::yield_now().await;
        let mut had_turn_done = false;
        loop {
            match turn_stream.try_recv() {
                Some(Ok(event)) => {
                    if matches!(event, TurnEventV1::TurnDone { .. }) {
                        had_turn_done = true;
                    }
                    {
                        let guard = notify_tx.lock().await;
                        if let Some(ref tx) = *guard {
                            if let Some(client_event) = turn_event_to_client_event(&event) {
                                if let Ok(json_str) = event_to_json(&client_event) {
                                    if tx.send(json_str).await.is_err() {
                                        debug!(
                                            "Event sink closed during drain, stopping event stream"
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Err(rejection)) => {
                    warn!(schema = %rejection.actual, "Schema mismatch in event drain, skipping");
                }
                None => break,
            }
        }
        if !had_turn_done {
            let guard = notify_tx.lock().await;
            if let Some(ref tx) = *guard {
                if tx.send(json!({"jsonrpc": "2.0", "method": "event", "params": {"type": "turn_done"}}).to_string()).await.is_err() {
                    warn!("Event sink closed, unable to send turn_done event");
                }
            }
        }

        let turn_succeeded = text.is_ok();
        let (text, metrics) = text.unwrap_or_else(|e| {
            (
                format!("error: {e}"),
                TurnMetrics {
                    tool_calls_made: 0,
                    tool_errors: 0,
                    elapsed_ms: 0,
                    iterations: 0,
                    completed_normally: false,
                },
            )
        });
        info!(len = text.len(), "ReAct loop completed");

        // -- Post-turn settlement --
        // Session gateway update
        {
            let tool_names: Vec<String> = tool_calls_for_session
                .iter()
                .map(|(_, name, _)| name.clone())
                .collect();
            self.session_gateway
                .update_turn_state(
                    turn_count,
                    metrics.tool_errors,
                    metrics.tool_calls_made,
                    tool_names,
                    self.runtime_ports.storm.failure_count().await,
                    Some(goal_message_for_gw.clone()),
                )
                .await;
        }

        let outcome_status = if turn_succeeded {
            fabric::dasein::OutcomeStatus::Succeeded
        } else {
            fabric::dasein::OutcomeStatus::Failed
        };
        self.runtime_ports
            .self_policy
            .coordinate(turn_count, &text, outcome_status)
            .await;

        self.runtime_ports
            .observability
            .record_turn(acc_tokens_in, acc_tokens_out);

        let msg_preview_end = message
            .char_indices()
            .nth(60)
            .map(|(i, _)| i)
            .unwrap_or(message.len());
        self.runtime_ports
            .self_policy
            .narrate(
                "chat_completed",
                &format!(
                    "User asked: '{}...' | Response: {} chars",
                    &message[..msg_preview_end],
                    text.len(),
                ),
            )
            .await;

        let turn = self
            .runtime_ports
            .sessions
            .finish(
                turn_succeeded,
                &tool_calls_for_session,
                &tool_results_for_session,
                &text,
            )
            .await?;

        // -- PR-3: drain the per-turn OperationScope (guarantees no orphan tasks) --
        {
            let mut guard = self.current_scope.lock().await;
            // TurnDone is streamed before post-turn hooks finish. A new client
            // can therefore begin another turn while this turn is winding
            // down. Never take and cancel that newer turn's global scope.
            let owns_current_scope = guard.as_ref().is_some_and(|scope| scope.id == operation_id);
            let owned_scope = owns_current_scope.then(|| guard.take()).flatten();
            if let Some(mut scope) = owned_scope {
                scope
                    .cancel_and_drain(self.clock.as_ref(), Duration::from_secs(5))
                    .await;
            }
        }

        Ok(json!({"jsonrpc": "2.0", "id": id, "result": {
            "response": text, "turn": turn, "succeeded": turn_succeeded,
                "metrics": {
                    "tool_calls_made": metrics.tool_calls_made,
                    "tool_errors": metrics.tool_errors,
                    "elapsed_ms": metrics.elapsed_ms,
                    "iterations": metrics.iterations,
                    "completed_normally": metrics.completed_normally
                },
                "canonical_items": canonical_items,
                "projection": {
                    "session_id": session_id_for_agora,
                    "agora_start_version": agora_start_version
                }
        }}))
    }
}

// ---------------------------------------------------------------------------
// TurnEventV1 conversion helpers (bridge from cognit::Event to fabric TurnEventV1)
// ---------------------------------------------------------------------------

/// Convert a `cognit::Event` into a `TurnEventV1` for EnvelopeV2 streaming.
fn convert_event_to_turn_event(event: Event) -> TurnEventV1 {
    match event {
        Event::TurnStarted { iteration } => TurnEventV1::TurnStarted { iteration },
        Event::TextDelta { delta } => TurnEventV1::TextDelta { delta },
        Event::ToolCallStart { name, call_id } => TurnEventV1::ToolCallStart { name, call_id },
        Event::ToolCallComplete {
            call_id,
            name,
            args,
        } => TurnEventV1::ToolCallComplete {
            call_id,
            name,
            args,
        },
        Event::ToolResult {
            name,
            call_id,
            result,
        } => TurnEventV1::ToolResult {
            name,
            call_id,
            content: result.content,
            is_error: result.is_error,
            execution_time_ms: result.execution_time_ms,
        },
        Event::Usage {
            tokens_in,
            tokens_out,
            cache_hit_tokens,
            cache_miss_tokens,
        } => TurnEventV1::Usage {
            tokens_in,
            tokens_out,
            cache_hit_tokens,
            cache_miss_tokens,
        },
        Event::TurnDone { result } => TurnEventV1::TurnDone {
            result: match result {
                Ok(text) => Some(text),
                Err(e) => Some(format!("error: {}", e)),
            },
        },
        Event::Error { message } => TurnEventV1::Error { message },
        Event::AwarenessChanged { level, context } => {
            TurnEventV1::AwarenessChanged { level, context }
        }
        Event::ModeChanged { mode } => TurnEventV1::ModeChanged { mode },
        Event::SubAgentStatusChanged {
            agent_id,
            status,
            task,
        } => TurnEventV1::SubAgentStatusChanged {
            agent_id,
            status,
            task,
        },
        Event::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        } => TurnEventV1::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        },
        Event::Interrupted { reason } => TurnEventV1::Interrupted { reason },
        Event::ContextUpdate {
            used_tokens,
            max_tokens,
        } => TurnEventV1::ContextUpdate {
            used_tokens,
            max_tokens,
        },
        Event::ModelSwitch { model_name } => TurnEventV1::ModelSwitch { model_name },
        Event::GoalSet { goal, sub_goals } => TurnEventV1::GoalSet { goal, sub_goals },
        Event::Reflection {
            summary,
            recommendation,
        } => TurnEventV1::Reflection {
            summary,
            recommendation,
        },
        Event::BudgetExceeded { used, max } => TurnEventV1::BudgetExceeded { used, max },
        Event::CircuitBreakerTripped { reason } => TurnEventV1::CircuitBreakerTripped { reason },
        Event::CompactionTriggered {
            used_tokens,
            threshold,
            reason,
        } => TurnEventV1::CompactionTriggered {
            used_tokens,
            threshold,
            reason,
        },
        Event::ApprovalRequest {
            id,
            tool,
            args,
            reason,
        } => TurnEventV1::Approval {
            id,
            tool,
            args,
            reason,
        },
        // Internal-only events → Generic catch-all (no serialization needed)
        _ => TurnEventV1::Generic {
            payload: serde_json::Value::Null,
        },
    }
}

/// Convert a `TurnEventV1` into a `ClientEvent` for TUI forwarding.
fn turn_event_to_client_event(event: &TurnEventV1) -> Option<ClientEvent> {
    match event {
        TurnEventV1::TurnStarted { iteration } => Some(ClientEvent::TurnStarted {
            iteration: *iteration,
        }),
        TurnEventV1::TextDelta { delta } => Some(ClientEvent::TextDelta {
            text: delta.clone(),
        }),
        TurnEventV1::ToolCallStart { name, call_id } => Some(ClientEvent::ToolCallStart {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: serde_json::Value::Null,
        }),
        TurnEventV1::ToolCallComplete {
            call_id,
            name,
            args,
        } => Some(ClientEvent::ToolCallComplete {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: args.clone(),
        }),
        TurnEventV1::ToolResult {
            name,
            call_id,
            content,
            is_error,
            execution_time_ms,
        } => Some(ClientEvent::ToolCallResult {
            call_id: call_id.clone(),
            tool: name.clone(),
            output: content.clone(),
            is_error: *is_error,
            elapsed_ms: *execution_time_ms,
        }),
        TurnEventV1::Usage {
            tokens_in,
            tokens_out,
            ..
        } => Some(ClientEvent::Usage {
            tokens_in: *tokens_in as u64,
            tokens_out: *tokens_out as u64,
        }),
        TurnEventV1::TurnDone { .. } => Some(ClientEvent::TurnDone),
        TurnEventV1::Error { message } => Some(ClientEvent::Error {
            message: message.clone(),
        }),
        TurnEventV1::AwarenessChanged { level, context } => Some(ClientEvent::AwarenessChanged {
            level: level.clone(),
            context: context.clone(),
        }),
        TurnEventV1::ModeChanged { mode } => Some(ClientEvent::ModeChanged { new: mode.clone() }),
        TurnEventV1::SubAgentStatusChanged {
            agent_id,
            status,
            task,
        } => Some(ClientEvent::SubAgentStatus {
            agent_id: agent_id.clone(),
            task: task.clone(),
            status: status.clone(),
        }),
        TurnEventV1::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        } => Some(ClientEvent::PlanUpdate {
            version: *version,
            plan: plan.clone(),
            critique: critique.clone(),
            ready_for_approval: *ready_for_approval,
        }),
        TurnEventV1::Interrupted { .. } => Some(ClientEvent::Interrupted),
        TurnEventV1::ContextUpdate {
            used_tokens,
            max_tokens,
        } => Some(ClientEvent::ContextUpdate {
            used_tokens: *used_tokens as u64,
            max_tokens: *max_tokens as u64,
        }),
        TurnEventV1::ModelSwitch { model_name } => Some(ClientEvent::ModelSwitch {
            model: model_name.clone(),
        }),
        TurnEventV1::GoalSet { goal, sub_goals } => Some(ClientEvent::GoalSet {
            goal: goal.clone(),
            sub_goals: sub_goals.clone(),
        }),
        TurnEventV1::Reflection { summary, .. } => Some(ClientEvent::Reflection {
            summary: summary.clone(),
        }),
        TurnEventV1::BudgetExceeded { max, .. } => {
            Some(ClientEvent::BudgetExceeded { limit: *max as u64 })
        }
        TurnEventV1::CircuitBreakerTripped { reason } => Some(ClientEvent::CircuitBreakerTripped {
            reason: reason.clone(),
        }),
        TurnEventV1::CompactionTriggered { .. } => Some(ClientEvent::CompactionTriggered),
        // TextDeltaStop, Approval, Generic → no client-facing event
        TurnEventV1::TextDeltaStop | TurnEventV1::Approval { .. } | TurnEventV1::Generic { .. } => {
            None
        }
    }
}

/// Serialize a `ClientEvent` into a JSON-RPC notification string.
fn event_to_json(event: &ClientEvent) -> serde_json::Result<String> {
    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": event,
    });
    serde_json::to_string(&notification)
}
