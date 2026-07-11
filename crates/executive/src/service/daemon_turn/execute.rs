//! execute_turn — the main orchestration entry point for daemon chat turns.
//!
//! This method ties together all the pre-turn injection, SelfField review,
//! tool setup, ReAct loop spawning, event pumping, and post-turn settlement
//! phases implemented across the sibling modules.

use super::super::daemon_react::{submit_streaming_daemon_turn, DaemonStreamingTurnContext};
use super::helpers::{bounded_text_history, build_request_messages};
use super::orchestrator::DaemonTurnOrchestrator;

use crate::r#impl::daemon::handler::tool_executor::TurnToolExecutor;
use cognit::harness::event_sink::{ChannelEventSink, Event};
use cognit::harness::linear::TurnMetrics;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::include::agora::AgoraOperation;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionRequest, CapabilityId, CapabilityScope, ContentBlock, Intent, IntentSource,
    LlmProvider, Message, OperationKind, OperationManager, OperationRequest, PrincipalId, Role,
    SandboxDecision, SandboxRequirement, TurnRequest, UsageReport,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

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

        let session_id = self.subsystems.default_session_id.lock().await.clone();

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
        let sf_ctx = fabric::Context::new(&session_id, std::env::current_dir().unwrap_or_default());
        let verdict = self.sf_review(&intent, &sf_ctx).await;

        // Sandbox requirement from SelfField verdict — passed to tool admission.
        let mut sandbox_requirement = SandboxRequirement::NotRequired;

        match verdict {
            Ok(fabric::Verdict::Deny { ref reason }) => {
                warn!(reason = %reason, "SelfField denied chat intent");
                self.sf_narrate("chat_denied", reason).await;
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32010, "message": format!("Intent denied by SelfField: {}", reason)}});
            }
            Ok(fabric::Verdict::SandboxFirst { ref reason }) => {
                warn!(reason = %reason, "SelfField requires sandbox; tools will be gated through admission");
                self.sf_narrate("chat_sandbox_required", reason).await;
                sandbox_requirement = SandboxRequirement::Required;
            }
            _ => {}
        }

        // -- Memory composition --
        let system_prompt = {
            let prefix = self.subsystems.cached_prefix.lock().await;
            prefix.clone()
        };
        let memory_block = self.compose_memory_block().await;
        {
            let mut sb = self.subsystems.storm_breaker.lock().await;
            sb.reset();
        }
        let mut effective_message = String::new();
        if !memory_block.is_empty() {
            effective_message.push_str(&memory_block);
            effective_message.push_str("\n\n");
        }
        if let Err(ref e) = verdict {
            warn!(error = %e, "SelfField review error, proceeding with caution");
        }
        self.inject_keyword_skills(message, &mut effective_message)
            .await;
        self.inject_fact_recall(message, &mut effective_message)
            .await;
        self.inject_core_memory(&mut effective_message).await;
        self.inject_skill_suggestion(message, &mut effective_message)
            .await;
        self.decay_stale_facts().await;

        // -- Configured pre_turn hook scripts --
        if !self.subsystems.hooks_config.pre_turn.is_empty() {
            let hook_session_id = self.get_or_create_session(None).await.0;
            let hook_input = serde_json::json!({"prompt": message, "session_id": hook_session_id});
            for script_path in &self.subsystems.hooks_config.pre_turn {
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
                        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
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

        effective_message.push_str(message);

        // -- PreTurn hooks --
        {
            let (sess_id, turn_count) = {
                let (_sid, sm_arc) = self.get_or_create_session(None).await;
                let sm = sm_arc.lock().await;
                (sm.session_id.clone(), sm.turn_count())
            };
            let hr = self.subsystems.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::PreTurn,
                session_id: sess_id,
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.to_string()),
                metadata: HashMap::new(),
            };
            match hr.execute(&ctx).await {
                HookResult::Block { reason } => {
                    warn!(reason = %reason, "PreTurn hook blocked");
                    return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32015, "message": format!("Blocked by hook: {}", reason)}});
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
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let mut sm = sm_arc.lock().await;
            sm.push_user(message).await;
        }
        // Persist user message to recall memory
        {
            let (sess_id, sm_arc) = self.get_or_create_session(None).await;
            let rm = self.subsystems.recall_memory.lock().await;
            let _ = rm.store(&sess_id, "user_message", message, None);
            let hr = self.subsystems.hook_registry.lock().await;
            let turn_count = sm_arc.lock().await.turn_count();
            let ctx = HookContext {
                point: HookPoint::OnMemoryStore,
                session_id: sess_id.clone(),
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.to_string()),
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        // -- Tool setup --
        let tool_defs = {
            let tools = self.subsystems.tools.lock().await;
            tools.definitions()
        };
        let working_dir = std::env::current_dir().unwrap_or_default();
        let (sess_id, sm_arc) = self.get_or_create_session(None).await;
        let turn_count = sm_arc.lock().await.turn_count();
        drop(sm_arc);

        // Agora seed — versioned publish via propose+commit (RFC-014 Phase 3B).
        let mut agora_version = self.subsystems.agora.version(&sess_id).await.unwrap_or(0);
        match self
            .subsystems
            .agora
            .propose(
                &sess_id,
                agora_version,
                AgoraOperation::PublishFact {
                    key: "turn_input".into(),
                    value: serde_json::json!(message),
                },
            )
            .await
        {
            Ok(prop) => {
                if let Err(e) = self.subsystems.agora.commit(&sess_id, prop.id).await {
                    tracing::warn!("agora commit (turn_input) failed: {e}");
                } else {
                    agora_version += 1;
                }
            }
            Err(e) => tracing::warn!("agora propose (turn_input) failed: {e}"),
        }

        let self_field_arc_for_react = self.subsystems.self_field.clone();
        let session_id_for_agora = sess_id.clone();

        let tool_executor = Arc::new(TurnToolExecutor::new(
            &self.subsystems,
            sess_id.clone(),
            turn_count,
            working_dir,
            operation.id,
            main_pid,
        ));

        // Admission controller for tool gating.
        let admission = self.subsystems.admission.clone();
        let adm_op_id = operation.id;
        let adm_pid = main_pid;
        let adm_sandbox = sandbox_requirement;

        let execute_tool = move |tool_id: &str, name: &str, input: &serde_json::Value| {
            let exec = tool_executor.clone();
            let adm = admission.clone();
            let (tid, n, inp) = (tool_id.to_string(), name.to_string(), input.clone());
            let op_id = adm_op_id;
            let pid = adm_pid;
            let sandbox_req = adm_sandbox;
            async move {
                // Build admission request for this tool invocation.
                let adm_req = AdmissionRequest {
                    operation_id: op_id,
                    process_id: pid,
                    principal: PrincipalId("agent".into()),
                    capability: CapabilityId(n.clone()),
                    action: n.clone(),
                    input_summary: format!("{:?}", inp).chars().take(200).collect(),
                    risk: RiskLevel::ReadOnly,
                    requested_scope: CapabilityScope::default(),
                    budget: None,
                    lease: None,
                    sandbox: sandbox_req,
                };

                // Admit — must pass admission gate.
                let permit = match adm.admit(adm_req).await {
                    Ok(p) => p,
                    Err(e) => return (format!("admission denied: {e}"), true),
                };

                // Check sandbox decision — fail closed.
                if matches!(permit.sandbox, SandboxDecision::Required) {
                    return (
                        format!(
                            "Sandbox required but execution infrastructure not available for '{n}'"
                        ),
                        true,
                    );
                }

                // Execute the tool through the existing pipeline.
                let (content, is_error) = exec.execute(&tid, &n, &inp).await;

                // Settle the permit (best-effort audit).
                let _ = adm
                    .settle(
                        permit.id,
                        UsageReport {
                            output_bytes: content.len() as u64,
                            exit_code: if is_error { Some(1) } else { Some(0) },
                            ..Default::default()
                        },
                    )
                    .await;

                (content, is_error)
            }
        };

        // LLM selection
        let task_type = self.model_router.classify_message(message);
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

        // Event channel
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(64);
        let event_sink = ChannelEventSink::new(event_tx);

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
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let mut sm = sm_arc.lock().await;
            let _ = sm.compact_if_needed(&*self.llm).await;
            let mut full_history = sm.history().to_vec();
            if full_history.last().is_some_and(|last| {
                last.role == Role::User
                    && last.content.iter().any(
                        |block| matches!(block, ContentBlock::Text { text } if text == message),
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
            },
        ));

        // -- Event + approval pumping loop --
        let approval_rx = self.subsystems.approval_rx.clone();
        let pending_approvals = self.subsystems.pending_approvals.clone();
        let notify_tx = self.notify_tx.clone();

        let mut tool_calls_for_session: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut tool_results_for_session: Vec<(String, String, bool)> = Vec::new();
        let mut acc_tokens_in: u64 = 0;
        let mut acc_tokens_out: u64 = 0;

        let text = loop {
            tokio::select! {
                result = &mut react_task => {
                    break result.unwrap_or_else(|e| Err(anyhow::anyhow!("react task panicked: {e}")));
                }
                Some(event) = event_rx.recv() => {
                    match &event {
                        Event::ToolCallStart { name, call_id } => {
                            tool_calls_for_session.push((call_id.clone(), name.clone(), serde_json::Value::Null));
                        }
                        Event::ToolCallComplete { call_id, name: _, args } => {
                            if let Some(tc) = tool_calls_for_session.iter_mut().find(|(id, _, _)| id == call_id) {
                                tc.2 = args.clone();
                            }
                        }
                        Event::ToolResult { name, call_id, result } => {
                            tool_results_for_session.push((call_id.clone(), result.content.clone(), result.is_error));
                            let evidence = fabric::Evidence::from_tool_result(
                                call_id.clone(), name.clone(), result.content.clone(), result.is_error,
                            );
                            let obs = serde_json::to_value(&evidence).unwrap_or(serde_json::Value::Null);
                            match self.subsystems.agora.propose(
                                &session_id_for_agora,
                                agora_version,
                                AgoraOperation::EmitObservation { obs },
                            ).await {
                                Ok(prop) => {
                                    if let Err(e) = self.subsystems.agora.commit(&session_id_for_agora, prop.id).await {
                                        tracing::warn!(target: "agora", error = %e, "agora commit (evidence) failed");
                                    } else {
                                        agora_version += 1;
                                    }
                                }
                                Err(e) => tracing::warn!(target: "agora", error = %e, "agora propose (evidence) failed"),
                            }
                        }
                        Event::Usage { tokens_in, tokens_out, .. } => {
                            acc_tokens_in += *tokens_in as u64;
                            acc_tokens_out += *tokens_out as u64;
                        }
                        _ => {}
                    }
                    // Forward event to TUI client
                    {
                        let guard = notify_tx.lock().await;
                        if let Some(ref tx) = *guard {
                            use crate::r#impl::daemon::handler::format::{event_to_client_event, event_to_json};
                            if let Some(client_event) = event_to_client_event(&event) {
                                if let Ok(json_str) = event_to_json(&client_event) {
                                    let _ = tx.send(json_str).await;
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

        // Drain remaining events
        let mut had_turn_done = false;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(event, Event::TurnDone { .. }) {
                had_turn_done = true;
            }
            {
                let guard = notify_tx.lock().await;
                if let Some(ref tx) = *guard {
                    use crate::r#impl::daemon::handler::format::{
                        event_to_client_event, event_to_json,
                    };
                    if let Some(client_event) = event_to_client_event(&event) {
                        if let Ok(json_str) = event_to_json(&client_event) {
                            let _ = tx.send(json_str).await;
                        }
                    }
                }
            }
        }
        if !had_turn_done {
            let guard = notify_tx.lock().await;
            if let Some(ref tx) = *guard {
                let _ = tx.send(json!({"jsonrpc": "2.0", "method": "event", "params": {"type": "turn_done"}}).to_string()).await;
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
            let sb = self.subsystems.storm_breaker.lock().await;
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

        if turn_succeeded {
            self.coordinate(&turn_count, &text).await;
        }

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
            self.extract_auto_memory(message, &text).await;
        }

        // Session history push
        let turn = if turn_succeeded {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
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
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let sm = sm_arc.lock().await;
            sm.turn_count()
        };

        if turn_succeeded {
            let sess_id = self.get_or_create_session(None).await.0;
            let rm = self.subsystems.recall_memory.lock().await;
            let _ = rm.store(&sess_id, "assistant_message", &text, None);
            let hr = self.subsystems.hook_registry.lock().await;
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
        self.commit_agora_snapshot(&session_id_for_agora, agora_version)
            .await;

        // -- Kernel: mark turn operation completed --
        let _ = self.operation_table.succeed(operation.id).await;

        json!({"jsonrpc": "2.0", "id": id, "result": {"response": text, "turn": turn}})
    }
}
