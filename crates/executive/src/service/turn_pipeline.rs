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
use cognit::CanonicalTurnEventSink;
use fabric::events::ui_event::ClientEvent;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::include::agora::{
    AgoraOperation, AgoraProposal, AgoraService, AgoraViewRequest, WorkspaceCommitPermit,
};
use fabric::ipc::{StreamConfig, TurnEventStream, TurnEventV1};
use fabric::CanonicalEventBus;
use fabric::{
    AgoraSpaceId, AgoraVersion, CapabilityCall, Clock, ContentBlock, ContextBinding, Intent,
    IntentSource, OperationId, PrincipalId, ProcessId, Role, SandboxRequirement, SessionId,
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
    pub(crate) agora: Option<Arc<dyn AgoraService>>,
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) current_scope: Arc<Mutex<Option<OperationScope>>>,
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
    pub(crate) context_assembler: Arc<crate::service::context_assembler::ContextAssembler>,
    pub(crate) canonical_sessions: Arc<crate::service::session_service::SessionService>,
    pub(crate) post_turn_projection:
        Arc<dyn crate::service::post_turn_projection::PostTurnProjection>,
    pub(crate) runtime_ports: Arc<crate::service::turn_runtime_ports::TurnRuntimePorts>,
    pub(crate) cognitive_sessions:
        Arc<dyn crate::service::harness_factory::CognitiveSessionFactory>,
    pub(crate) conscious_core:
        Option<Arc<dyn crate::service::conscious_workspace::ConsciousTurnPort>>,
    pub(crate) session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
    pub(crate) prompt_queue_enabled: bool,
    pub(crate) workspace_checkpoint:
        Arc<crate::service::workspace_checkpoint::WorkspaceCheckpointService>,
    pub(crate) lifecycle: Arc<crate::service::lifecycle_contributors::LifecycleRegistry>,
    pub(crate) lifecycle_enabled: bool,
    pub(crate) event_bus: Option<Arc<CanonicalEventBus>>,
}

pub(crate) struct TurnPipelineResources {
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) agora: Option<Arc<dyn AgoraService>>,
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) current_scope: Arc<Mutex<Option<OperationScope>>>,
    pub(crate) daemon_cancel: Option<CancellationToken>,
    pub(crate) context: Arc<crate::service::context_assembler::ContextAssembler>,
    pub(crate) canonical_sessions: Arc<crate::service::session_service::SessionService>,
    pub(crate) projection: Arc<dyn crate::service::post_turn_projection::PostTurnProjection>,
    pub(crate) runtime: Arc<crate::service::turn_runtime_ports::TurnRuntimePorts>,
    pub(crate) cognitive_sessions:
        Arc<dyn crate::service::harness_factory::CognitiveSessionFactory>,
    pub(crate) conscious_core:
        Option<Arc<dyn crate::service::conscious_workspace::ConsciousTurnPort>>,
    pub(crate) session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
    pub(crate) prompt_queue_enabled: bool,
    pub(crate) workspace_checkpoint:
        Arc<crate::service::workspace_checkpoint::WorkspaceCheckpointService>,
    pub(crate) lifecycle: Arc<crate::service::lifecycle_contributors::LifecycleRegistry>,
    pub(crate) lifecycle_enabled: bool,
    pub(crate) event_bus: Option<Arc<CanonicalEventBus>>,
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
            cognitive_sessions: resources.cognitive_sessions,
            conscious_core: resources.conscious_core,
            session_input: resources.session_input,
            prompt_queue_enabled: resources.prompt_queue_enabled,
            workspace_checkpoint: resources.workspace_checkpoint,
            lifecycle: resources.lifecycle,
            lifecycle_enabled: resources.lifecycle_enabled,
            event_bus: resources.event_bus,
        }
    }

    async fn dispatch_lifecycle(
        &self,
        input: crate::service::lifecycle_contributors::LifecycleInput,
        cancel: &CancellationToken,
    ) -> anyhow::Result<crate::service::lifecycle_contributors::LifecycleDispatch> {
        use crate::service::lifecycle_contributors::LifecycleEffect;

        let target = format!("thread:{}", input.thread_id.0);
        let dispatch = self
            .lifecycle
            .dispatch_if_enabled(self.lifecycle_enabled, input)
            .await?;
        for effect in &dispatch.effects {
            match effect {
                LifecycleEffect::EmitEvent { schema, payload } => {
                    if let Some(bus) = &self.event_bus {
                        let _ = bus
                            .publish_event(
                                fabric::SchemaId::from(schema.as_str()),
                                &target,
                                payload.clone(),
                            )
                            .await;
                    }
                }
                LifecycleEffect::RequestCancellation { .. } => cancel.cancel(),
                _ => {}
            }
            if let Some(bus) = &self.event_bus {
                let _ = bus
                    .publish_event(
                        fabric::SchemaId::from("aletheon.event.lifecycle_effect_applied/v1"),
                        &target,
                        serde_json::json!({"effect": format!("{effect:?}")}),
                    )
                    .await;
            }
        }
        Ok(dispatch)
    }

    async fn journal_protocol_turn_event(
        &self,
        session_id: &str,
        turn_id: fabric::TurnId,
        assistant_item_id: &str,
        event: &TurnEventV1,
    ) -> anyhow::Result<()> {
        use fabric::protocol::client::ItemPhase;
        let session_id = fabric::SessionId(session_id.to_owned());
        let entry = match event {
            TurnEventV1::TurnStarted { .. } => Some((
                assistant_item_id.to_owned(),
                ItemPhase::Started,
                None,
                Some(format!("{assistant_item_id}:assistant-started")),
            )),
            TurnEventV1::TextDelta { delta } => Some((
                assistant_item_id.to_owned(),
                ItemPhase::Streaming,
                Some(delta.clone()),
                None,
            )),
            TurnEventV1::ToolCallStart { call_id, .. } => {
                let id = format!("tool:{}:{call_id}", turn_id.0);
                Some((
                    id.clone(),
                    ItemPhase::Started,
                    None,
                    Some(format!("{id}:tool-started")),
                ))
            }
            TurnEventV1::ToolProgress {
                call_id, payload, ..
            } => Some((
                format!("tool:{}:{call_id}", turn_id.0),
                ItemPhase::Streaming,
                Some(payload.to_string()),
                None,
            )),
            _ => None,
        };
        if let Some((item_id, phase, delta, dedupe_key)) = entry {
            self.canonical_sessions
                .append_protocol_item_event(
                    &session_id,
                    item_id,
                    phase,
                    delta,
                    None,
                    None,
                    dedupe_key,
                )
                .await?;
        }
        Ok(())
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
        // Resolve the authoritative runtime session before policy review. The
        // read is non-mutating; denied turns still never call `begin_user`.
        let (session_id, current_turn_count) = self.runtime_ports.sessions.current().await?;

        let checkpoint_id = self
            .workspace_checkpoint
            .begin_turn(
                crate::service::workspace_checkpoint::CheckpointTurnContext {
                    session_id: session_id.clone(),
                    thread_id: turn_request.context.thread_id.0.clone(),
                    turn_id: turn_request.context.turn_id.as_ref().map_or_else(
                        || operation_id.0.to_string(),
                        |turn_id| turn_id.0.to_string(),
                    ),
                    prompt_index: current_turn_count as u64 + 1,
                    principal_id: principal.clone(),
                    workspace: fabric::types::workspace_checkpoint::WorkspaceIdentity {
                        canonical_path: turn_request.context.workspace.cwd().to_path_buf(),
                        repo_fingerprint: None,
                    },
                    writable_roots: turn_request.context.workspace.writable_roots().to_vec(),
                    created_at_ms: self.clock.wall_now().0,
                },
            )
            .await?;

        let lifecycle_principal = principal.clone();
        let lifecycle_thread = turn_request.context.thread_id.clone();
        let lifecycle_turn = turn_request.context.turn_id;
        let lifecycle_session = session_id.clone();
        let assistant_protocol_item = format!(
            "turn:{}:assistant",
            lifecycle_turn.unwrap_or(fabric::TurnId(operation_id.0)).0
        );
        let turn_result: anyhow::Result<serde_json::Value> = async {

        let lifecycle_start = self
            .dispatch_lifecycle(
                crate::service::lifecycle_contributors::LifecycleInput {
                    phase: crate::service::lifecycle_contributors::LifecyclePhase::BeforeTurnInput,
                    principal_id: lifecycle_principal.clone(),
                    thread_id: lifecycle_thread.clone(),
                    turn_id: lifecycle_turn,
                    session_id: lifecycle_session.clone(),
                    detail: serde_json::json!({"message": message}),
                },
                &scope_token,
            )
            .await?;

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
        let sf_ctx = fabric::Context::new(
            &session_id,
            turn_request.context.workspace.cwd().to_path_buf(),
        );
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
        let mut durable_fragments = Vec::new();
        for effect in &lifecycle_start.effects {
            if let crate::service::lifecycle_contributors::LifecycleEffect::AddContextFragment {
                source,
                content,
            } = effect
            {
                effective_message.push_str(&format!("[lifecycle:{source}]\n{content}\n"));
                durable_fragments.push((source.clone(), content.clone()));
            }
        }
        if !durable_fragments.is_empty() {
            let turn_id = lifecycle_turn
                .ok_or_else(|| anyhow::anyhow!("lifecycle context requires an explicit turn_id"))?;
            self.canonical_sessions
                .persist_context_fragments(
                    &fabric::SessionId(turn_request.context.thread_id.0.clone()),
                    turn_id,
                    fabric::types::lifecycle::LifecyclePhase::BeforeTurnInput,
                    durable_fragments,
                )
                .await?;
        }

        // -- PreTurn hooks (including configured scripts owned by Corpus) --
        {
            let authority_metadata = HashMap::from([
                (
                    "workspace_root".into(),
                    turn_request.context.workspace.cwd().display().to_string(),
                ),
                (
                    "repo_hooks_trusted".into(),
                    turn_request.context.repo_hooks_trusted.to_string(),
                ),
            ]);
            self.runtime_ports
                .hooks
                .execute(HookContext {
                    point: HookPoint::UserPromptSubmit,
                    session_id: session_id.clone(),
                    turn_count: current_turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: Some(message.clone()),
                    metadata: authority_metadata.clone(),
                })
                .await;
            let ctx = HookContext {
                point: HookPoint::PreTurn,
                session_id: session_id.clone(),
                turn_count: current_turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.clone()),
                metadata: authority_metadata,
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
        effective_message.push_str(&message);

        let (sess_id, turn_count) = self.runtime_ports.sessions.begin_user(&message).await?;

        if let Some(conscious) = &self.conscious_core {
            conscious
                .observe_turn(
                    AgoraSpaceId(sess_id.clone()),
                    main_pid,
                    main_pid,
                    operation_id,
                    &message,
                )
                .await?;
        }

        // Canonical Session/Turn/Item history is the only model replay source.
        let existing_messages = {
            let mut full_history = self
                .canonical_sessions
                .resume(&fabric::SessionId(turn_request.context.thread_id.0.clone()))
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
        let assembled_context = self
            .context_assembler
            .assemble(&context_request, &existing_messages)
            .await?;
        let context_projection_receipt = assembled_context.projection_receipt;
        let request_messages = assembled_context.messages;

        // LLM selection
        let llm = self.runtime_ports.models.select(&message);

        // -- Governed capability setup --
        // Context Space seed — user turn input is private overlay data, not
        // shared Agora fact. Shared visibility requires an explicit proposal.
        let agora = self.agora.clone();
        let mut agora_version = if let Some(ref agora) = agora {
            agora
                .view(AgoraViewRequest {
                    space: AgoraSpaceId(sess_id.clone()),
                })
                .await
                .map(|view| view.version)
                .unwrap_or(0)
        } else {
            tracing::warn!(target: "agora", "DomainPorts.agora is not configured; shared evidence commits disabled for this turn");
            0
        };
        let agora_start_version = agora_version;
        // Phase 2a: reuse the main agent's long-lived process space (one per
        // session, not per turn). Bindings are upserted so the Agora version is
        // refreshed in place rather than accumulating. Space is released on
        // process exit (see orchestrator::exit_process).
        let (agent_space, main_agent_id) = match self.kernel.inspect_process(main_pid).await {
            Ok(snapshot) => (snapshot.space, Some(snapshot.agent_id)),
            Err(e) => {
                tracing::warn!(target: "space", error = %e, "inspect(main_pid) failed; using ephemeral space for this turn");
                (SpaceId::new(), None)
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

        let action_loop = match &self.conscious_core {
            Some(resolver) => Some(
                resolver
                    .resolve(AgoraSpaceId(sess_id.clone()), main_pid, main_pid)
                    .await?,
            ),
            None => None,
        };
        let batch_planner = match &self.conscious_core {
            Some(conscious) => Some(
                conscious
                    .batch_planner(AgoraSpaceId(sess_id.clone()))
                    .await?,
            ),
            None => None,
        };
        // Create the canonical stream before capability composition so G2
        // progress and cognitive events share the same per-turn spine.
        let (mut turn_stream, turn_sender) = TurnEventStream::new(StreamConfig::turn_events(64));
        let config = self.runtime_ports.config.config().await;
        let before_tools = self
            .dispatch_lifecycle(
                crate::service::lifecycle_contributors::LifecycleInput {
                    phase: crate::service::lifecycle_contributors::LifecyclePhase::BeforeToolBatch,
                    principal_id: lifecycle_principal.clone(),
                    thread_id: lifecycle_thread.clone(),
                    turn_id: lifecycle_turn,
                    session_id: lifecycle_session.clone(),
                    detail: serde_json::json!({"tool_count": 0}),
                },
                &scope_token,
            )
            .await?;
        if let Some(reason) = before_tools.effects.iter().find_map(|effect| match effect {
            crate::service::lifecycle_contributors::LifecycleEffect::RejectInput { reason } => {
                Some(reason)
            }
            _ => None,
        }) {
            anyhow::bail!("lifecycle contributor rejected tool batch: {reason}");
        }
        let prepared =
            self.runtime_ports
                .capabilities
                .prepare(CapabilityExecutionContext {
                    agent: main_agent_id.map(|agent_id| fabric::AgentToolContext {
                        caller_root_agent_id: agent_id,
                        parent_agent_id: agent_id,
                        parent_process_id: main_pid,
                    }),
                    process_id: main_pid,
                    operation_id,
                    principal: principal.clone(),
                    connection_id: turn_request.context.connection_id.clone(),
                    thread_id: turn_request.context.thread_id.clone(),
                    turn_id: turn_request.context.turn_id.ok_or_else(|| {
                        anyhow::anyhow!("turn capability authority has no TurnId")
                    })?,
                    workspace: turn_request.context.workspace.clone(),
                    session_id: sess_id.clone(),
                    working_dir: turn_request.context.workspace.cwd().to_path_buf(),
                    sandbox: sandbox_requirement,
                    cancel: scope_token.clone(),
                    turn_count,
                    repo_hooks_trusted: turn_request.context.repo_hooks_trusted,
                    action_loop,
                    streaming_tools: config.streaming_tools,
                    turn_event_sender: Some(turn_sender.clone()),
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

        let event_sink = CanonicalTurnEventSink::new(turn_sender);

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
                cancel_token: scope_token.clone(),
                sessions: self.cognitive_sessions.clone(),
                batch_planner,
                session_input: self.session_input.clone(),
                prompt_queue_enabled: self.prompt_queue_enabled,
            },
        ));

        // -- Event + approval pumping loop --
        let notify_tx = self.notify_tx.clone();

        let mut tool_calls_for_session: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut tool_results_for_session: Vec<(String, String, bool)> = Vec::new();
        let mut canonical_items: Vec<fabric::ItemPayload> = Vec::new();
        let mut acc_tokens_in: u64 = 0;
        let mut acc_tokens_out: u64 = 0;
        let mut terminal_events = TerminalEventBuffer::default();

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
                    self.journal_protocol_turn_event(
                        &lifecycle_session,
                        lifecycle_turn.unwrap_or(fabric::TurnId(operation_id.0)),
                        &assistant_protocol_item,
                        &event,
                    ).await?;
                    let is_terminal = terminal_events.observe(&event);
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
                                let proposal = AgoraProposal {
                                    id: uuid::Uuid::new_v4(),
                                    space: AgoraSpaceId(session_id_for_agora.clone()),
                                    author: main_pid,
                                    base_version: agora_version,
                                    operation: AgoraOperation::AcceptEvidence { evidence },
                                    evidence: vec![format!("tool-call:{call_id}")],
                                    confidence: if *is_error { 0.5 } else { 1.0 },
                                    expires_at_ms: Some(
                                        clock_for_agora.wall_now().0.saturating_add(30_000),
                                    ),
                                };
                                let permit = WorkspaceCommitPermit::issue_for(
                                    &proposal,
                                    clock_for_agora.wall_now().0.saturating_add(30_000),
                                );
                                match (agora.propose(proposal.clone()).await, permit) {
                                    (Ok(id), Ok(permit)) => {
                                        let result = agora.commit(id, permit).await;
                                        if let Err(e) = result {
                                            tracing::warn!(target: "agora", error = %e, "agora commit (evidence) failed");
                                        } else {
                                            agora_version += 1;
                                        }
                                    }
                                    (Err(e), _) => tracing::warn!(target: "agora", error = %e, "agora propose (evidence) failed"),
                                    (_, Err(e)) => tracing::warn!(target: "agora", error = %e, "agora permit issue failed"),
                                }
                            } else {
                                tracing::warn!(
                                    target: "agora",
                                    "DomainPorts.agora missing; skipping shared evidence commit"
                                );
                            }
                            if let Err(error) = self.dispatch_lifecycle(
                                crate::service::lifecycle_contributors::LifecycleInput {
                                    phase: crate::service::lifecycle_contributors::LifecyclePhase::AfterToolTerminal,
                                    principal_id: lifecycle_principal.clone(),
                                    thread_id: lifecycle_thread.clone(),
                                    turn_id: lifecycle_turn,
                                    session_id: lifecycle_session.clone(),
                                    detail: serde_json::json!({
                                        "call_id": call_id,
                                        "tool_name": name,
                                        "is_error": is_error,
                                    }),
                                },
                                &scope_token,
                            ).await {
                                scope_token.cancel();
                                react_task.abort();
                                let _ = (&mut react_task).await;
                                return Err(error);
                            }
                        }
                        TurnEventV1::Usage { tokens_in, tokens_out, .. } => {
                            acc_tokens_in += *tokens_in as u64;
                            acc_tokens_out += *tokens_out as u64;
                        }
                        _ => {}
                    }
                    // Forward event to TUI client
                    if !is_terminal {
                        let sender = notify_tx.lock().await.clone();
                        if let Some(tx) = sender {
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
                        let sender = notify_tx.lock().await.clone();
                        if let Some(tx) = sender {
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
        loop {
            match turn_stream.try_recv() {
                Some(Ok(event)) => {
                    self.journal_protocol_turn_event(
                        &lifecycle_session,
                        lifecycle_turn.unwrap_or(fabric::TurnId(operation_id.0)),
                        &assistant_protocol_item,
                        &event,
                    ).await?;
                    let is_terminal = terminal_events.observe(&event);
                    if !is_terminal {
                        let sender = notify_tx.lock().await.clone();
                        if let Some(tx) = sender {
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

        // Terminal events are buffered while the ReAct task is running so a
        // failed task always produces one ordered Error -> TurnDone sequence.
        // Successful tasks produce exactly one TurnDone even when Cognit also
        // reported completion before its task joined.
        let turn_error = text.as_ref().err().map(ToString::to_string);
        let normalized_terminal_events = terminal_events.into_client_events(turn_error);
        {
            let sender = notify_tx.lock().await.clone();
            if let Some(tx) = sender {
                for event in normalized_terminal_events {
                    let Ok(json_str) = event_to_json(&event) else {
                        warn!("Unable to serialize terminal turn event");
                        continue;
                    };
                    if tx.send(json_str).await.is_err() {
                        warn!("Event sink closed, unable to send terminal turn event");
                        break;
                    }
                }
            }
        }

        let turn_succeeded = text.is_ok();
        let result = text.unwrap_or_else(|e| fabric::TurnResult {
            output: format!("error: {e}"),
            stop: fabric::TurnStop::Failed,
            metrics: fabric::TurnMetrics {
                tool_calls_made: 0,
                tool_errors: 0,
                elapsed_ms: 0,
                iterations: 0,
                completed_normally: false,
            },
        });
        let text = result.output;
        let metrics = result.metrics;
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

        self.dispatch_lifecycle(
            crate::service::lifecycle_contributors::LifecycleInput {
                phase: crate::service::lifecycle_contributors::LifecyclePhase::AfterTurnTerminal,
                principal_id: lifecycle_principal.clone(),
                thread_id: lifecycle_thread.clone(),
                turn_id: lifecycle_turn,
                session_id: lifecycle_session.clone(),
                detail: serde_json::json!({"succeeded": turn_succeeded}),
            },
            &scope_token,
        )
        .await?;

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
                    "agora_start_version": agora_start_version,
                    "conscious_context": context_projection_receipt
                }
        }}))
        }
        .await;

        if turn_result.is_err() {
            let _ = self
                .dispatch_lifecycle(
                    crate::service::lifecycle_contributors::LifecycleInput {
                        phase: crate::service::lifecycle_contributors::LifecyclePhase::OnAbort,
                        principal_id: lifecycle_principal,
                        thread_id: lifecycle_thread,
                        turn_id: lifecycle_turn,
                        session_id: lifecycle_session,
                        detail: serde_json::json!({"reason": "turn pipeline error"}),
                    },
                    &scope_token,
                )
                .await;
        }

        if let Some(checkpoint_id) = checkpoint_id {
            let succeeded = turn_result
                .as_ref()
                .ok()
                .and_then(|response| response.get("result"))
                .and_then(|result| result.get("succeeded"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            self.workspace_checkpoint
                .finalize_turn(checkpoint_id, succeeded)
                .await?;
        }
        turn_result
    }
}

#[derive(Default)]
struct TerminalEventBuffer {
    error: Option<String>,
    turn_done: bool,
}

impl TerminalEventBuffer {
    /// Buffer terminal stream events instead of forwarding them immediately.
    /// This lets the join result normalize failures without duplicate or
    /// out-of-order completion notifications.
    fn observe(&mut self, event: &TurnEventV1) -> bool {
        match event {
            TurnEventV1::Error { message } => {
                if self.error.is_none() {
                    self.error = Some(message.clone());
                }
                true
            }
            TurnEventV1::TurnDone { .. } => {
                self.turn_done = true;
                true
            }
            _ => false,
        }
    }

    fn into_client_events(self, turn_error: Option<String>) -> Vec<ClientEvent> {
        let error = self.error.or(turn_error);
        if !self.turn_done {
            debug!("Synthesizing missing terminal turn_done event");
        }

        let mut events = Vec::with_capacity(if error.is_some() { 2 } else { 1 });
        if let Some(message) = error {
            events.push(ClientEvent::Error { message });
        }
        events.push(ClientEvent::TurnDone);
        events
    }
}

/// Convert a `TurnEventV1` into a `ClientEvent` for TUI forwarding.
/// Project the canonical daemon turn stream into the legacy TUI wire event.
/// Public so transport/consumer integration tests exercise the production
/// projection rather than duplicating its field mapping.
pub fn turn_event_to_client_event(event: &TurnEventV1) -> Option<ClientEvent> {
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
        TurnEventV1::ToolProgress {
            name,
            call_id,
            kind,
            payload,
        } => Some(ClientEvent::ToolProgress {
            call_id: call_id.clone(),
            tool: name.clone(),
            kind: kind.clone(),
            payload: payload.clone(),
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
        // Outcome remains on the canonical turn stream for audit/metrics; the
        // legacy UI only has a generic triggered marker.
        TurnEventV1::CompactionOutcome { .. }
        | TurnEventV1::TextDeltaStop
        | TurnEventV1::Approval { .. }
        | TurnEventV1::Generic { .. } => None,
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

#[cfg(test)]
mod terminal_event_tests {
    use super::*;

    #[tokio::test]
    async fn real_bash_progress_crosses_guard_bridge_and_client_projection() {
        use corpus::security::approval::AutoApproveGate;
        use corpus::security::sandbox::executor::SandboxPreference;
        use corpus::{AuditLogger, ToolRunnerWithGuard};
        use fabric::ToolContext;

        let temp = tempfile::tempdir().unwrap();
        let clock: Arc<dyn fabric::Clock> = Arc::new(aletheon_kernel::chronos::SystemClock::new());
        let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
            AuditLogger::new(temp.path().join("audit.jsonl")).unwrap(),
            SandboxPreference::Forbid,
            clock.clone(),
        )
        .with_approval_gate(Arc::new(AutoApproveGate));
        let context = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "g2-client-stream".into(),
            clock,
        };
        let (mut sink, event_rx) = fabric::tool_event_channel();

        let report = runner
            .execute_tool_streaming_report(
                &corpus::tools::bash_exec::BashExecTool,
                serde_json::json!({
                    "command": "printf 'alpha\\n'; sleep 0.02; printf 'beta\\n'"
                }),
                &context,
                "g2-turn",
                &mut sink,
            )
            .await;
        assert!(
            report.result.is_ok(),
            "real BashExecTool must settle successfully"
        );
        drop(sink);

        let (mut turn_stream, turn_sender) =
            fabric::ipc::TurnEventStream::new(StreamConfig::turn_events(8));
        let outcome = crate::service::tool_stream_bridge::bridge_tool_stream(
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
