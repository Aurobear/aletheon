//! TurnPipeline — shared turn orchestration for daemon and exec paths.
//!
//! Extracted from DaemonTurnOrchestrator::execute_turn so both the daemon
//! and CLI exec paths share the same Pre/Cognit/Post turn pipeline.

use crate::core::core_systems::CoreSystems;
use crate::core::session_gateway::SessionGateway;
use crate::r#impl::daemon::handler::tool_executor::TurnToolExecutor;
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::r#impl::daemon::session_manager::SessionManager;
use crate::service::daemon_react::{submit_streaming_daemon_turn, DaemonStreamingTurnContext};
use crate::service::daemon_turn::helpers::{bounded_text_history, build_request_messages};
use crate::service::governed_capability::{CapabilityRuntimeFactory, RegistryAuthorityProvider};
use aletheon_kernel::chronos::SystemTimer;
use aletheon_kernel::operation::{OperationScope, OperationTable};
use aletheon_kernel::process::ProcessTable;
use aletheon_kernel::space::InMemorySpaceManager;
use aletheon_kernel::supervision::SupervisorTree;
use cognit::harness::event_sink::{ChannelEventSink, Event};
use cognit::harness::linear::TurnMetrics;
use fabric::events::ui_event::ClientEvent;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::include::agora::{AgoraOperation, WorkspaceCommitPermit};
use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::ipc::{StreamConfig, TurnEventStream, TurnEventV1};
use fabric::{
    AdmissionController, AgoraOps, AgoraSpaceId, AgoraVersion, CapabilityCall, Clock, ContentBlock,
    ContextBinding, Intent, IntentSource, LlmProvider, Message, OperationId, PrincipalId,
    ProcessId, ProcessManager, Role, SandboxRequirement, SessionId, SpaceId, Timer, TurnRequest,
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
    pub(crate) subsystems: Arc<CoreSystems>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) llm: Arc<dyn LlmProvider>,
    pub(crate) model_router: Arc<ModelRouter>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) agora: Option<Arc<dyn AgoraOps>>,
    pub(crate) mailbox_service: Arc<InProcessMailboxService>,
    pub(crate) process_table: Arc<ProcessTable>,
    pub(crate) operation_table: Arc<OperationTable>,
    pub(crate) space_manager: Arc<InMemorySpaceManager>,
    pub(crate) supervisor: Arc<Mutex<SupervisorTree>>,
    pub(crate) current_scope: Arc<Mutex<Option<OperationScope>>>,
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
}

impl TurnPipeline {
    /// Create a new TurnPipeline, cloning all handles from the service ports.
    pub fn new(
        subsystems: Arc<CoreSystems>,
        sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
        session_gateway: Arc<SessionGateway>,
        llm: Arc<dyn LlmProvider>,
        model_router: Arc<ModelRouter>,
        notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
        daemon_cancel_token: Option<CancellationToken>,
    ) -> Self {
        let clock = subsystems.ports.clock.clone();
        let process_table = subsystems.ports.process_table.clone();
        let operation_table = subsystems.ports.operation_table.clone();
        let supervisor = subsystems.ports.supervisor.clone();
        let admission = subsystems.ports.admission.clone();
        let agora = subsystems.ports.agora.clone();
        let mailbox_service = subsystems.ports.mailbox_service.clone();
        let space_manager = subsystems.ports.space_manager.clone();

        Self {
            subsystems,
            sessions,
            session_gateway,
            llm,
            model_router,
            notify_tx,
            clock,
            admission,
            agora,
            mailbox_service,
            process_table,
            operation_table,
            space_manager,
            supervisor,
            current_scope: Arc::new(Mutex::new(None)),
            daemon_cancel_token,
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
        let verdict = self.sf_review(&intent, &sf_ctx).await;

        // Sandbox requirement from SelfField verdict — passed to tool admission.
        let mut sandbox_requirement = SandboxRequirement::NotRequired;

        match verdict {
            Ok(fabric::Verdict::Deny { ref reason }) => {
                warn!(reason = %reason, "SelfField denied chat intent");
                self.sf_narrate("chat_denied", reason).await;
                return Ok(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32010, "message": format!("Intent denied by SelfField: {}", reason)}}),
                );
            }
            Ok(fabric::Verdict::SandboxFirst { ref reason }) => {
                warn!(reason = %reason, "SelfField requires sandbox; tools will be gated through admission");
                self.sf_narrate("chat_sandbox_required", reason).await;
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

        // -- Memory composition --
        let mut system_prompt = {
            let prefix = self.subsystems.session.cached_prefix.lock().await;
            prefix.clone()
        };
        system_prompt.push_str(&format!(
            "\n\nCurrent working directory: {}\nTreat this as the user's current project. Do not scan unrelated host directories to guess a project.",
            turn_request.working_dir.display()
        ));
        let memory_block = self.compose_memory_block().await;
        {
            let mut sb = self.subsystems.security.storm_breaker.lock().await;
            sb.reset();
        }
        let mut effective_message = String::new();
        if !memory_block.is_empty() {
            effective_message.push_str(&memory_block);
            effective_message.push_str("\n\n");
        }
        self.inject_keyword_skills(&message, &mut effective_message)
            .await;
        self.inject_composite_recall(&message, &turn_request.session_id, &mut effective_message)
            .await;
        self.inject_core_memory(&mut effective_message).await;
        self.inject_skill_suggestion(&message, &mut effective_message)
            .await;
        self.decay_stale_facts().await;

        // -- Configured pre_turn hook scripts --
        if !self.subsystems.corpus.hooks_config.pre_turn.is_empty() {
            let hook_session_id = self.get_or_create_session(None).await?.0;
            let hook_input = serde_json::json!({"prompt": message, "session_id": hook_session_id});
            for script_path in &self.subsystems.corpus.hooks_config.pre_turn {
                let path = crate::r#impl::daemon::handler::format::expand_tilde(script_path);
                if !std::path::Path::new(&path).exists() {
                    tracing::warn!(path = %path, "Hook script not found, skipping");
                    continue;
                }
                let spawn_result = tokio::process::Command::new(&path)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                match spawn_result {
                    Ok(mut child) => {
                        if let Some(stdin) = child.stdin.take() {
                            let input = hook_input.to_string();
                            tokio::spawn(async move {
                                use tokio::io::AsyncWriteExt;
                                let mut stdin = stdin;
                                let _ = stdin.write_all(input.as_bytes()).await;
                            });
                        }
                        let mut stdout_pipe = child.stdout.take();
                        match SystemTimer
                            .timeout(std::time::Duration::from_secs(30), child.wait())
                            .await
                        {
                            Ok(Ok(status)) if status.success() => {
                                if let Some(ref mut stdout) = stdout_pipe {
                                    use tokio::io::AsyncReadExt;
                                    let mut buf = String::new();
                                    if stdout.read_to_string(&mut buf).await.is_ok()
                                        && !buf.is_empty()
                                    {
                                        effective_message
                                            .push_str(&format!("\n[Hook output]\n{}\n", buf));
                                    }
                                }
                            }
                            Ok(Ok(status)) => {
                                tracing::warn!(path = %path, code = status.code(), "Hook script exited with non-zero status");
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(path = %path, error = %e, "Hook script I/O error");
                            }
                            Err(_) => {
                                tracing::warn!(path = %path, "Hook script timed out (30s)");
                                child.kill().await.ok();
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(path = %path, error = %e, "Failed to spawn hook script");
                    }
                }
            }
        }

        effective_message.push_str(&message);

        // -- PreTurn hooks --
        {
            let (sess_id, turn_count) = {
                let (_sid, sm_arc) = self.get_or_create_session(None).await?;
                let sm = sm_arc.lock().await;
                (sm.session_id.clone(), sm.turn_count())
            };
            let hr = self.subsystems.corpus.hook_registry.lock().await;
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
            match hr.execute(&ctx).await {
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

        // Push user message into session history
        {
            let (_sid, sm_arc) = self.get_or_create_session(None).await?;
            let mut sm = sm_arc.lock().await;
            sm.push_user(&message).await;
        }
        // Persist user message to recall memory
        {
            let (sess_id, sm_arc) = self.get_or_create_session(None).await?;
            let turn_count = sm_arc.lock().await.turn_count();
            let observed_at = fabric::wall_to_datetime(self.clock.wall_now());
            let _ = self
                .subsystems
                .memory
                .memory_service
                .record(mnemosyne::ExperienceEvent::Message {
                    session: sess_id.clone(),
                    role: "user".into(),
                    content: message.clone(),
                    metadata: mnemosyne::MemoryMetadata::local(
                        format!("message:{sess_id}:user:{turn_count}"),
                        format!("{sess_id}:user:{turn_count}"),
                        observed_at,
                    ),
                })
                .await;
            let hr = self.subsystems.corpus.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnMemoryStore,
                session_id: sess_id.clone(),
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.clone()),
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        // -- Tool setup --
        let tool_defs = {
            let tools = self.subsystems.corpus.tools.lock().await;
            tools.definitions()
        };
        let working_dir = turn_request.working_dir.clone();
        let (sess_id, sm_arc) = self.get_or_create_session(None).await?;
        let turn_count = sm_arc.lock().await.turn_count();
        drop(sm_arc);

        // Context Space seed — user turn input is private overlay data, not
        // shared Agora fact. Shared visibility requires an explicit proposal.
        let agora = self.agora.clone();
        let mut agora_version = if let Some(ref agora) = agora {
            agora.version(&sess_id).await.unwrap_or(0)
        } else {
            tracing::warn!(target: "agora", "ServicePorts.agora is not configured; shared evidence commits disabled for this turn");
            0
        };
        let agora_start_version = agora_version;
        // Phase 2a: reuse the main agent's long-lived process space (one per
        // session, not per turn). Bindings are upserted so the Agora version is
        // refreshed in place rather than accumulating. Space is released on
        // process exit (see orchestrator::exit_process).
        let agent_space = match self.process_table.inspect(main_pid).await {
            Ok(snap) => snap.space,
            Err(e) => {
                tracing::warn!(target: "space", error = %e, "inspect(main_pid) failed; using ephemeral space for this turn");
                SpaceId::new()
            }
        };
        self.space_manager.upsert_binding(
            agent_space,
            ContextBinding::Session(SessionId(sess_id.clone())),
        );
        self.space_manager.upsert_binding(
            agent_space,
            ContextBinding::Agora(AgoraSpaceId(sess_id.clone()), AgoraVersion(agora_version)),
        );
        if let Err(e) =
            self.space_manager
                .set_overlay(agent_space, "turn_input", serde_json::json!(message))
        {
            tracing::warn!(target: "space", error = %e, "failed to store turn input overlay");
        }

        let self_field_arc_for_react = self.subsystems.self_field.clone();
        let session_id_for_agora = sess_id.clone();
        let agora_for_events = agora.clone();
        let clock_for_agora = self.clock.clone();

        let tool_executor = Arc::new(TurnToolExecutor::new(
            &self.subsystems,
            sess_id.clone(),
            turn_count,
            working_dir.clone(),
            operation_id,
            main_pid,
        ));

        let authority = Arc::new(RegistryAuthorityProvider::new(
            corpus::tool_risk_levels(&self.subsystems.corpus.tools).await,
            principal,
            sess_id.clone(),
            working_dir,
            sandbox_requirement,
            scope_token.clone(),
        ));
        let capability =
            CapabilityRuntimeFactory::build(self.admission.clone(), tool_executor, authority);

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

        // LLM selection
        let task_type = self.model_router.classify_message(&message);
        let llm: Arc<dyn LlmProvider> = match self.model_router.create_provider(task_type) {
            Ok(provider) => {
                info!(task = ?task_type, model = provider.name(), "Model selected by router");
                Arc::from(provider)
            }
            Err(e) => {
                warn!(error = %e, task = ?task_type, "ModelRouter failed, falling back to default");
                self.llm.clone()
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

        // Dasein injection
        let effective_message = {
            let sf = self.subsystems.self_field.lock().await;
            if let Some(ctx) = sf.dasein_prompt_injection() {
                format!("{}\n\n---\n\n{}", ctx, effective_message)
            } else {
                effective_message
            }
        };

        let config = self.subsystems.runtime.lock().await.config().clone();
        let goal_message = message.to_string();
        let goal_message_for_gw = goal_message.clone();

        // History compaction
        let existing_messages = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await?;
            let mut sm = sm_arc.lock().await;
            let _ = sm.compact_if_needed(&*self.llm).await;
            let mut full_history = sm.history().to_vec();
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

        let request_messages =
            build_request_messages(system_prompt, &existing_messages, effective_message);

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
                self_field: self_field_arc_for_react,
                cancel_token: scope_token,
            },
        ));

        // -- Event + approval pumping loop --
        let approval_rx = self.subsystems.security.approval_rx.clone();
        let pending_approvals = self.subsystems.security.pending_approvals.clone();
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
                                    "ServicePorts.agora missing; skipping shared evidence commit"
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
                Some(pending) = async {
                    let mut rx = approval_rx.lock().await;
                    rx.recv().await
                } => {
                    let approval_id = uuid::Uuid::new_v4().to_string();
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "approval_request",
                        "params": {
                            "approval_id": approval_id,
                            "tool": pending.request.tool,
                            "action_summary": pending.request.action_summary,
                            "risk_level": pending.request.risk_level,
                            "detail": pending.request.detail,
                        }
                    });
                    {
                        let mut map = pending_approvals.lock().await;
                        map.insert(approval_id.clone(), pending.respond);
                    }
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
            let sb = self.subsystems.security.storm_breaker.lock().await;
            self.session_gateway
                .update_turn_state(
                    turn_count,
                    metrics.tool_errors,
                    metrics.tool_calls_made,
                    tool_names,
                    sb.failure_count(),
                    Some(goal_message_for_gw.clone()),
                )
                .await;
        }

        let outcome_status = if turn_succeeded {
            fabric::dasein::OutcomeStatus::Succeeded
        } else {
            fabric::dasein::OutcomeStatus::Failed
        };
        self.coordinate(&turn_count, &text, outcome_status).await;

        self.subsystems
            .debug_perf
            .record_turn(acc_tokens_in, acc_tokens_out);

        let msg_preview_end = message
            .char_indices()
            .nth(60)
            .map(|(i, _)| i)
            .unwrap_or(message.len());
        self.sf_narrate(
            "chat_completed",
            &format!(
                "User asked: '{}...' | Response: {} chars",
                &message[..msg_preview_end],
                text.len(),
            ),
        )
        .await;

        self.run_post_turn_hooks().await;
        if turn_succeeded {
            self.extract_auto_memory(&message, &text).await;
        }

        // Session history push
        let turn = if turn_succeeded {
            let (_sid, sm_arc) = self.get_or_create_session(None).await?;
            let mut sm = sm_arc.lock().await;
            if !tool_calls_for_session.is_empty() {
                let content_blocks: Vec<ContentBlock> = tool_calls_for_session
                    .iter()
                    .map(|(tid, name, input)| ContentBlock::ToolUse {
                        id: tid.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    })
                    .collect();
                sm.push_message(Message {
                    role: Role::Assistant,
                    content: content_blocks,
                })
                .await;
                let result_blocks: Vec<ContentBlock> = tool_results_for_session
                    .iter()
                    .map(|(call_id, content, is_error)| ContentBlock::ToolResult {
                        tool_use_id: call_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    })
                    .collect();
                sm.push_message(Message {
                    role: Role::User,
                    content: result_blocks,
                })
                .await;
            }
            sm.push_assistant(&text).await;
            let _ = sm.compact_if_needed(&*self.llm).await;
            sm.turn_count()
        } else {
            let (_sid, sm_arc) = self.get_or_create_session(None).await?;
            let sm = sm_arc.lock().await;
            sm.turn_count()
        };

        if turn_succeeded {
            let sess_id = self.get_or_create_session(None).await?.0;
            let observed_at = fabric::wall_to_datetime(self.clock.wall_now());
            let _ = self
                .subsystems
                .memory
                .memory_service
                .record(mnemosyne::ExperienceEvent::Message {
                    session: sess_id.clone(),
                    role: "assistant".into(),
                    content: text.clone(),
                    metadata: mnemosyne::MemoryMetadata::local(
                        format!("message:{sess_id}:assistant:{turn}"),
                        format!("{sess_id}:assistant:{turn}"),
                        observed_at,
                    ),
                })
                .await;
            let hr = self.subsystems.corpus.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnMemoryStore,
                session_id: sess_id.clone(),
                turn_count: turn,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        let task_summary = if message.len() > 100 {
            let end = message
                .char_indices()
                .nth(100)
                .map(|(i, _)| i)
                .unwrap_or(message.len());
            format!("{}...", &message[..end])
        } else {
            message.to_string()
        };
        self.record_turn_reflection(&task_summary, &text, turn)
            .await;
        self.run_post_evolution(&task_summary, &text, &metrics)
            .await;
        self.commit_agora_snapshot(&session_id_for_agora, agora_start_version)
            .await;

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
                "canonical_items": canonical_items
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
