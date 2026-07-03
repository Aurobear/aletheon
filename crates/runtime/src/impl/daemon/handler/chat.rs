#![allow(deprecated)]
// TODO(P1-A): See handler/mod.rs for migration notes. This file's allow(deprecated)
// is inherited from mod.rs's event_bus field (Arc<dyn EventBus>) used for
// DaseinEventBridge wiring. Once mod.rs is migrated, this file can drop the allow.

use super::format::event_to_json;
use super::RequestHandler;

use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use base::hook::{HookContext, HookPoint, HookResult};
use base::{Context as AbiContext, Intent, IntentSource, ReflectionTrigger, SelfFieldOps, Verdict};
use std::collections::HashMap;

use crate::core::event_sink::{ChannelEventSink, Event, EventSink};
use crate::core::react_loop::ReActLoop;
use crate::r#impl::memory::fact_store::FactStore;
use cognit::r#impl::llm::LlmProvider;

impl RequestHandler {
    pub(super) async fn handle_chat(
        &self,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let message = request["params"]["message"].as_str().unwrap_or("");
        info!(message = %message, "Chat request received");

        // Create a fresh per-turn cancellation token so the daemon can
        // cancel this turn during graceful shutdown.
        let _turn_token = self.begin_turn_token().await;

        // --- SelfField review: gate the user message before LLM ---
        let intent = Intent {
            action: "chat".to_string(),
            parameters: serde_json::json!({ "message": message }),
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
        let sf_ctx = AbiContext::new(
            &self.state.lock().await.runtime.config().session_id,
            std::env::current_dir().unwrap_or_default(),
        );

        let verdict = self.sf_review(&intent, &sf_ctx).await;

        match verdict {
            Ok(Verdict::Deny { ref reason }) => {
                warn!(reason = %reason, "SelfField denied chat intent");
                self.sf_narrate("chat_denied", reason).await;
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32010, "message": format!("Intent denied by SelfField: {}", reason) }
                });
            }
            _ => {} // SandboxFirst and other verdicts handled below in user turn
        }

        // Use the cache-stable prefix (built once at boot)
        let system_prompt = {
            let prefix = self.cached_prefix.lock().await;
            prefix.clone()
        };

        // Build effective user message with memory updates and SandboxFirst note.
        // Both go into the user turn to preserve cache-stable system prompt.
        // Memory updates are composed via compose_memory_block() — the same
        // pattern as ReActLoop::compose_user_message() / Controller::compose_user_message().
        let memory_block = self.compose_memory_block().await;
        let mut effective_message = String::new();

        // Memory updates first (if any)
        if !memory_block.is_empty() {
            effective_message.push_str(&memory_block);
            effective_message.push_str("\n\n");
        }

        // SelfField SandboxFirst note (if flagged) — injected into user turn
        if let Ok(Verdict::SandboxFirst { ref reason }) = verdict {
            info!(reason = %reason, "SelfField flagged chat for sandbox");
            effective_message.push_str(&format!(
                "<selffield-note>SandboxFirst: This interaction has been flagged for sandbox review. Reason: {}</selffield-note>\n\n",
                reason
            ));
        } else if let Err(ref e) = verdict {
            warn!(error = %e, "SelfField review error, proceeding with caution");
        }

        // --- Keyword skill injection ---
        // Gather loaded skills with keywords and match against user message.
        {
            let loader = self.skill_loader.lock().await;
            let skill_keywords: Vec<crate::r#impl::skills::keyword_matcher::SkillKeywords> = loader
                .plugins()
                .iter()
                .filter(|p| !p.keywords.is_empty())
                .map(|p| crate::r#impl::skills::keyword_matcher::SkillKeywords {
                    name: p.name.clone(),
                    keywords: p.keywords.clone(),
                    body: p.system_prompt.clone(),
                })
                .collect();
            drop(loader);
            let matched =
                crate::r#impl::skills::keyword_matcher::match_skills(message, &skill_keywords);
            for body in matched {
                effective_message.push_str("\n<activated-skill>\n");
                effective_message.push_str(&body);
                effective_message.push_str("\n</activated-skill>\n");
            }
        }

        // --- Fact recall from FactStore ---
        {
            let fs = self.fact_store.lock().await;
            let keywords: Vec<String> = message
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .map(|w| w.to_lowercase())
                .collect();
            let query = keywords.join(" ");
            if query.len() >= 8 {
                if let Ok(facts) = fs.search_facts_governed(&query, None, false, 0.15, 4) {
                    if !facts.is_empty() {
                        let mut recall_block = String::from("\n[Recalled memories]\n");
                        for fact in &facts {
                            recall_block.push_str(&format!(
                                "- {} (trust: {:.2})\n",
                                fact.content, fact.trust_score
                            ));
                            let _ = fs.record_feedback(fact.fact_id, true);
                        }
                        // Entity graph boost
                        let entities = FactStore::extract_entities(message);
                        for entity in entities.iter().take(3) {
                            if let Ok(eid) = fs.resolve_entity(entity) {
                                if let Ok(related) = fs.get_entity_facts(eid) {
                                    for rf in related.iter().take(1) {
                                        if !facts.iter().any(|f| f.fact_id == rf.fact_id) {
                                            recall_block.push_str(&format!(
                                                "- {} (entity: {})\n",
                                                rf.content, entity
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        info!(count = facts.len(), "Fact recall injected");
                        effective_message.push_str(&recall_block);
                    }
                }
            }
        }

        // --- Inject current CoreMemory state ---
        // CoreMemory is baked into the system prompt prefix at boot, but
        // core_memory_append/AutoMemory updates it in-memory after that.
        // Inject the current state so the model sees up-to-date facts.
        {
            let cm = self.core_memory.lock().await;
            let mut core_lines = Vec::new();
            for (label, block) in cm.blocks() {
                if block.read_only || block.value.is_empty() {
                    continue;
                }
                // Only inject non-empty, writable blocks (human, learned, etc.)
                for line in block.value.lines() {
                    if !line.trim().is_empty() {
                        core_lines.push(format!("[core:{}] {}", label, line));
                    }
                }
            }
            if !core_lines.is_empty() {
                effective_message.push_str("\n[Core Memory — current state]\n");
                for line in &core_lines {
                    effective_message.push_str(line);
                    effective_message.push('\n');
                }
            }
        }

        // --- Skill suggestion via SkillRouter ---
        {
            let sr = self.skill_router.lock().await;
            let suggestions = sr.suggest(message, 0.6, 1);
            if let Some(suggestion) = suggestions.first() {
                info!(skill = %suggestion.name, confidence = suggestion.confidence, "Skill suggested");
                effective_message.push_str(&format!(
                    "\n[Suggested skill] /{} (confidence: {:.2}) — {}\n",
                    suggestion.name, suggestion.confidence, suggestion.description
                ));
            }
        }

        // --- Periodic stale fact decay ---
        {
            let fs = self.fact_store.lock().await;
            let _ = fs.decay_stale();
        }

        // --- Configured pre_turn hook scripts ---
        if !self.hooks_config.pre_turn.is_empty() {
            let hook_session_id = self.get_or_create_session(None).await.0;
            let hook_input = serde_json::json!({
                "prompt": message,
                "session_id": hook_session_id
            });
            let hook_outputs = self
                .run_hook_scripts(&self.hooks_config.pre_turn, &hook_input.to_string())
                .await;
            for output in hook_outputs {
                effective_message.push_str(&format!("\n[Hook output]\n{}\n", output));
            }
        }

        effective_message.push_str(message);

        // --- PreTurn hooks ---
        {
            // Gather session info before locking hook_registry
            let (session_id, turn_count) = {
                let (_sid, sm_arc) = self.get_or_create_session(None).await;
                let sm = sm_arc.lock().await;
                (sm.session_id.clone(), sm.turn_count())
            };
            let hr = self.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::PreTurn,
                session_id,
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
                    return json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": {"code": -32015, "message": format!("Blocked by hook: {}", reason)}
                    });
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
            if sm.turn_count() == 0 {
                sm.push_system(&system_prompt);
            }
            sm.push_user(&effective_message).await;
        }
        // Persist user message to recall memory
        {
            let (session_id, sm_arc) = self.get_or_create_session(None).await;
            let rm = self.recall_memory.lock().await;
            let _ = rm.store(&session_id, "user_message", message, None);
            // Fire OnMemoryStore hook
            {
                let hr = self.hook_registry.lock().await;
                let turn_count = sm_arc.lock().await.turn_count();
                let ctx = HookContext {
                    point: HookPoint::OnMemoryStore,
                    session_id: session_id.clone(),
                    turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: Some(message.to_string()),
                    metadata: HashMap::new(),
                };
                hr.execute(&ctx).await;
            }
        }

        // --- Interleaved ReAct loop with tools ---
        // Build tool definitions from the shared tool registry.
        let tool_defs = {
            let tools = self.tools.lock().await;
            tools.definitions()
        };

        // Prepare execute_tool closure that runs tools through the guarded runner.
        let runner = self.tool_runner.clone();
        let tools_arc = self.tools.clone();
        let hook_registry_arc = self.hook_registry.clone();
        let storm_breaker_arc = self.storm_breaker.clone();
        let memory_queue_arc = self.memory_queue.clone();
        let session_approvals_arc = self.session_approvals.clone();
        let notify_tx_arc = self.notify_tx.clone();
        let debug_perf_arc = self.debug_perf.clone();
        let self_field_arc = self.self_field.clone();
        let working_dir = std::env::current_dir().unwrap_or_default();
        let (session_id, sm_arc) = self.get_or_create_session(None).await;
        let turn_count = sm_arc.lock().await.turn_count();
        drop(sm_arc);

        // Clone self_field_arc before the execute_tool closure moves it,
        // so the tokio::spawn block below can also use it for per-turn Dasein injections.
        let self_field_arc_for_react = self_field_arc.clone();

        let execute_tool = move |id: &str, name: &str, input: &serde_json::Value| {
            let runner = runner.clone();
            let tools_arc = tools_arc.clone();
            let hook_registry_arc = hook_registry_arc.clone();
            let storm_breaker_arc = storm_breaker_arc.clone();
            let memory_queue_arc = memory_queue_arc.clone();
            let session_approvals_arc = session_approvals_arc.clone();
            let _notify_tx_arc = notify_tx_arc.clone();
            let debug_perf = debug_perf_arc.clone();
            let self_field_arc = self_field_arc.clone();
            let _call_id = id.to_string();
            let name = name.to_string();
            let input = input.clone();
            let working_dir = working_dir.clone();
            let session_id = session_id.clone();
            let turn_count = turn_count;
            async move {
                // --- PreTool hook ---
                {
                    let hr = hook_registry_arc.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::PreTool,
                        session_id: session_id.clone(),
                        turn_count,
                        tool_name: Some(name.clone()),
                        tool_input: Some(input.clone()),
                        tool_result: None,
                        message: None,
                        metadata: HashMap::new(),
                    };
                    if let HookResult::Block { reason } = hr.execute(&ctx).await {
                        return (format!("Blocked by hook: {}", reason), true);
                    }
                }

                // --- OnMemoryRecall hook (when memory_search tool is invoked) ---
                if name == "memory_search" {
                    let hr = hook_registry_arc.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::OnMemoryRecall,
                        session_id: session_id.clone(),
                        turn_count,
                        tool_name: Some(name.clone()),
                        tool_input: Some(input.clone()),
                        tool_result: None,
                        message: None,
                        metadata: HashMap::new(),
                    };
                    hr.execute(&ctx).await;
                }

                // --- Check session approvals (auto-approve if "always" was used) ---
                {
                    let approvals = session_approvals_arc.lock().await;
                    if let Some(&approved) = approvals.get(&name) {
                        if approved {
                            info!(tool = %name, "Auto-approving tool from session approval cache");
                        }
                    }
                }

                // SelfField review per-tool
                {
                    let tool_intent = Intent {
                        action: name.clone(),
                        parameters: input.clone(),
                        source: IntentSource::Body,
                        description: format!("Tool call: {}", name),
                    };
                    let sf_ctx = AbiContext::new(&session_id, working_dir.clone());
                    let sf = self_field_arc.lock().await;
                    match sf.review(&tool_intent, &sf_ctx).await {
                        Ok(Verdict::Deny { reason }) => {
                            let _ = sf
                                .narrate("tool_blocked", &format!("{}: {}", name, reason))
                                .await;
                            return (format!("Tool blocked by SelfField: {}", reason), true);
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                tool = %name,
                                "SelfField review error, proceeding"
                            );
                        }
                        _ => {}
                    }
                }

                let tool = {
                    let reg = tools_arc.lock().await;
                    reg.get(&name).cloned()
                };
                let exec_ctx = base::tool::ToolContext {
                    working_dir,
                    session_id: session_id.clone(),
                };
                let (content, is_error) = match tool {
                    Some(t) => {
                        let mut r = runner.lock().await;
                        let res = r
                            .run(t.as_ref(), input.clone(), &exec_ctx, "chat-turn")
                            .await;
                        (res.content, res.is_error)
                    }
                    None => (format!("Unknown tool: {}", name), true),
                };

                // --- PerfCounter: record tool call and errors ---
                debug_perf.record_tool_call(&name).await;
                if is_error {
                    debug_perf.record_error();
                }

                // --- StormBreaker: track consecutive failures ---
                {
                    let mut sb = storm_breaker_arc.lock().await;
                    if let Some(directive) = sb.record(&name, is_error, &content) {
                        let mut mq = memory_queue_arc.lock().await;
                        mq.push(format!("\n[Storm Breaker] {}\n", directive));
                    }
                }

                // --- PostTool hook ---
                {
                    let hr = hook_registry_arc.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::PostTool,
                        session_id,
                        turn_count,
                        tool_name: Some(name.clone()),
                        tool_input: None,
                        tool_result: Some(base::hook::HookToolResult {
                            content: content.clone(),
                            is_error,
                            execution_time_ms: 0,
                        }),
                        message: None,
                        metadata: HashMap::new(),
                    };
                    hr.execute(&ctx).await;
                }

                // tool_call_result is emitted via EventSink in ReActLoop (single source of truth).
                (content, is_error)
            }
        };

        // Drive the ReAct loop.  SelfField review already ran above,
        // so the inner review_fn returns Allow to avoid double-gating.
        //
        // We spawn the ReAct loop as a background task so we can
        // concurrently pump approval requests from the SocketApprovalGate.
        let approval_rx = self.approval_rx.clone();
        let pending_approvals = self.pending_approvals.clone();
        let notify_tx = self.notify_tx.clone();

        // Dynamic model selection based on message content
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

        // Create event channel for streaming ReAct loop events.
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(64);
        let event_sink = ChannelEventSink::new(event_tx);

        // Inject Dasein context into user input (Task 17)
        let effective_message = {
            let sf = self.self_field.lock().await;
            if let Some(ctx) = sf.dasein_prompt_injection() {
                format!("{}\n\n---\n\n{}", ctx, effective_message)
            } else {
                effective_message
            }
        };

        let config = self.state.lock().await.runtime.config().clone();
        let llm_clone = llm.clone();
        let tool_defs_clone = tool_defs.clone();
        let goal_message = message.to_string();
        let goal_message_for_gw = goal_message.clone();

        // Pre-turn proactive compaction: compact the persisted history before
        // seeding the ReAct loop so the seed is already within token budget.
        let existing_messages = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let mut sm = sm_arc.lock().await;
            let _ = sm.compact_if_needed(&*self.llm).await;
            sm.history().to_vec()
        };

        let mut react_task = tokio::spawn(async move {
            let mut react_loop = ReActLoop::new(config);
            // Seed with existing conversation history for context continuity
            react_loop.seed_messages(existing_messages);
            react_loop.set_goal(goal_message.clone());
            let sf_for_ctx = self_field_arc_for_react.clone();
            react_loop.set_dasein_context_provider(Box::new(move || {
                sf_for_ctx
                    .try_lock()
                    .ok()
                    .and_then(|sf| sf.dasein_prompt_injection())
            }));
            event_sink.emit(Event::GoalSet {
                goal: goal_message,
                sub_goals: vec![],
            });
            react_loop
                .run_streaming(
                    &effective_message,
                    &*llm_clone,
                    &tool_defs_clone,
                    execute_tool,
                    &event_sink,
                )
                .await
        });

        // Pump approval requests and streaming events while the ReAct loop is running.
        // When a tool needs L2+ approval, the SocketApprovalGate sends
        // a PendingApproval through the channel. We generate an
        // approval_id, store the oneshot sender, and notify the client.
        let mut tool_calls_for_session: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut tool_results_for_session: Vec<(String, String, bool)> = Vec::new();

        let text = loop {
            tokio::select! {
                result = &mut react_task => {
                    // ReAct loop finished — drain any remaining approvals
                    // (they get auto-denied by the 120s timeout in the gate).
                    break result.unwrap_or_else(|e| Err(anyhow::anyhow!("react task panicked: {e}")));
                }
                Some(event) = event_rx.recv() => {
                    // Track tool calls for session manager
                    match &event {
                        Event::ToolCallStart { name, call_id } => {
                            tool_calls_for_session.push((call_id.clone(), name.clone(), serde_json::Value::Null));
                        }
                        Event::ToolCallComplete { call_id, name: _, args } => {
                            // Update the tool call with its actual arguments
                            if let Some(tc) = tool_calls_for_session.iter_mut().find(|(id, _, _)| id == call_id) {
                                tc.2 = args.clone();
                            }
                        }
                        Event::ToolResult { name: _, call_id, result } => {
                            tool_results_for_session.push((call_id.clone(), result.content.clone(), result.is_error));
                        }
                        _ => {}
                    }

                    if let Some(json_str) = event_to_json(&event) {
                        if let Some(ref tx) = notify_tx {
                            let _ = tx.send(json_str).await;
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

                    // Store the oneshot sender so approval_response can resolve it.
                    {
                        let mut map = pending_approvals.lock().await;
                        map.insert(approval_id.clone(), pending.respond);
                    }

                    // Send notification to client.
                    if let Some(ref tx) = notify_tx {
                        if tx.send(notification.to_string()).await.is_err() {
                            warn!("Failed to send approval_request notification — client disconnected?");
                        }
                    } else {
                        warn!("No notify_tx configured — approval request will timeout (fail-safe deny)");
                    }
                }
            }
        };

        // Drain remaining events from the ReAct loop (including turn_done).
        // The select! loop breaks as soon as react_task completes, but the
        // event channel may still have pending events (especially turn_done
        // which is the last event emitted by the ReAct loop).
        let mut had_turn_done = false;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(event, Event::TurnDone { .. }) {
                had_turn_done = true;
            }
            if let Some(json_str) = event_to_json(&event) {
                if let Some(ref tx) = notify_tx {
                    let _ = tx.send(json_str).await;
                }
            }
        }

        // If the turn was cancelled, send a synthetic turn_done event
        // so the TUI transitions out of the streaming state.
        if !had_turn_done {
            if let Some(ref tx) = notify_tx {
                let _ = tx
                    .send(
                        json!({
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {"type": "turn_done"}
                        })
                        .to_string(),
                    )
                    .await;
            }
        }

        let (text, metrics) = text.unwrap_or_else(|e| {
            (
                format!("error: {e}"),
                crate::core::react_loop::TurnMetrics {
                    tool_calls_made: 0,
                    tool_errors: 0,
                    elapsed_ms: 0,
                    iterations: 0,
                    completed_normally: false,
                },
            )
        });
        info!(len = text.len(), "ReAct loop completed");

        // Update SessionGateway state with current turn metrics for external debug access.
        {
            let tool_names: Vec<String> = tool_calls_for_session
                .iter()
                .map(|(_, name, _)| name.clone())
                .collect();
            let sb = self.storm_breaker.lock().await;
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

        // Coordinate: quick mood update after the turn (Task 23)
        self.coordinate(&turn_count, &text).await;

        // Record turn in perf counter (token counts come from usage events
        // which are not captured here; use 0 as placeholder).
        self.debug_perf.record_turn(0, 0);

        // Narrate the completed interaction in the SelfField narrative layer (bus-aware)
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

        // --- PostTurn hooks ---
        {
            // Gather session info before locking hook_registry
            let (session_id, turn_count) = {
                let (_sid, sm_arc) = self.get_or_create_session(None).await;
                let sm = sm_arc.lock().await;
                (sm.session_id.clone(), sm.turn_count())
            };
            let hr = self.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::PostTurn,
                session_id,
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        // --- Auto-memory extraction ---
        // Runs after PostTurn hooks, before compaction.
        // Uses a cheap LLM to extract facts from the turn.
        {
            let mut am = self.auto_memory.lock().await;
            if let Ok(facts) = am.analyze_and_store(message, &text).await {
                if !facts.is_empty() {
                    info!(count = facts.len(), "Auto-memory: stored facts");
                }
            }
        }

        // Push tool call messages and assistant response to session manager
        // This ensures the session history has the correct structure for OpenAI API
        let turn = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let mut sm = sm_arc.lock().await;

            // Push tool call messages if any
            if !tool_calls_for_session.is_empty() {
                use base::message::{ContentBlock, Message, Role};

                // Create assistant message with tool_use blocks
                let content_blocks: Vec<ContentBlock> = tool_calls_for_session
                    .iter()
                    .map(|(id, name, input)| ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    })
                    .collect();
                sm.push_message(Message {
                    role: Role::Assistant,
                    content: content_blocks,
                });

                // Push tool result messages
                for (call_id, content, is_error) in &tool_results_for_session {
                    sm.push_message(Message::tool_result(call_id, content, *is_error));
                }
            }

            sm.push_assistant(&text).await;
            let _ = sm.compact_if_needed(&*self.llm).await;
            sm.turn_count()
        };
        // Persist assistant response to recall memory
        {
            let session_id = self.get_or_create_session(None).await.0;
            let rm = self.recall_memory.lock().await;
            let _ = rm.store(&session_id, "assistant_message", &text, None);
            // Fire OnMemoryStore hook
            {
                let hr = self.hook_registry.lock().await;
                let ctx = HookContext {
                    point: HookPoint::OnMemoryStore,
                    session_id: session_id.clone(),
                    turn_count: turn,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: None,
                    metadata: HashMap::new(),
                };
                hr.execute(&ctx).await;
            }
        }

        // Enhanced reflection: analyze question and response quality
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

        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();

        // Response length as a quality indicator
        let resp_len = text.len();
        if resp_len > 500 {
            what_worked.push(format!("Detailed response ({} chars)", resp_len));
        } else if resp_len > 100 {
            what_worked.push(format!("Concise response ({} chars)", resp_len));
        } else {
            what_worked.push(format!("Brief response ({} chars)", resp_len));
        }

        // Detect error indicators in response
        let text_lower = text.to_lowercase();
        let error_indicators = [
            "error",
            "failed",
            "unable",
            "cannot",
            "couldn't",
            "sorry, i",
            "i don't know",
        ];
        for indicator in &error_indicators {
            if text_lower.contains(indicator) {
                what_failed.push(format!("Response contains '{}'", indicator));
            }
        }

        // Detect learning/self-correction indicators
        let learning_indicators = [
            "i learned",
            "i now understand",
            "i realize",
            "correction:",
            "actually,",
        ];
        for indicator in &learning_indicators {
            if text_lower.contains(indicator) {
                learned.push(format!("Self-correction detected: '{}'", indicator));
            }
        }

        // Track turn context
        what_worked.push(format!("Conversation turn #{}", turn));

        let has_failures = !what_failed.is_empty();
        let entry = self.reflector.reflect_conversation(
            &task_summary,
            ReflectionTrigger::TaskComplete,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        // Store reflection — drop lock guard before re-locking for evolution check
        let store_result = {
            let mem = self.episodic_memory.lock().await;
            mem.store_reflection(&entry)
        };
        if let Err(e) = store_result {
            warn!(error = %e, "Failed to store chat reflection");
        } else {
            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");

            // Periodic evolution trigger: every 10 reflections, run ExperienceSummarizer
            let mem = self.episodic_memory.lock().await;
            if let Ok(count) = mem.reflection_count() {
                if count > 0 && count % 10 == 0 {
                    info!(
                        count = count,
                        "Running ExperienceSummarizer (periodic trigger)"
                    );
                    if let Ok(recent) = mem.recall_reflections(20) {
                        if let Some(evo_entry) =
                            cognit::core::ExperienceSummarizer::summarize(&recent)
                        {
                            if let Err(e) = mem.store_evolution_log(&evo_entry) {
                                warn!(error = %e, "Failed to store evolution log");
                            } else {
                                info!(id = %evo_entry.id, patterns = evo_entry.patterns_detected.len(), "Evolution log stored");
                            }
                        }
                    }
                }
            }
        }

        // EvolutionCoordinator: post-turn evolution (accumulates reflections, triggers every N turns)
        {
            let success = metrics.completed_normally && !text.starts_with("error:");
            let mut state = self.state.lock().await;
            if let Err(e) = state
                .runtime
                .post_evolution(
                    &task_summary,
                    &text,
                    success,
                    metrics.tool_calls_made,
                    metrics.tool_errors,
                    metrics.elapsed_ms,
                    metrics.iterations,
                    &*self.pipeline,
                )
                .await
            {
                warn!(error = %e, "post_evolution failed");
            }
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "response": text, "turn": turn }
        })
    }
}
