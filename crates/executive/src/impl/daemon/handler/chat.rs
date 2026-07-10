// Handler module migrated to CommunicationBus — event_bus field is now Arc<CommunicationBus>.

use super::format::{event_to_client_event, event_to_json};
use super::RequestHandler;

use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::ops::AgoraOps;
use fabric::{
    ContentBlock, Context as AbiContext, Intent, IntentSource, Message, ReflectionTrigger, Role,
    SelfFieldOps, Verdict,
};
use std::collections::HashMap;

use cognit::harness::config::HarnessConfig;
use cognit::harness::event_sink::{ChannelEventSink, Event, EventSink};
use cognit::harness::linear::DynLlmRef;
use cognit::harness::linear::ReActLoop;
use cognit::harness::linear::TurnMetrics;
use cognit::r#impl::llm::LlmProvider;
use mnemosyne::AdvancedCompressor;
use mnemosyne::FactStore;

const MAX_ACTIVATED_SKILL_CHARS: usize = 12 * 1024;
const MAX_ACTIVATED_SKILLS_TOTAL_CHARS: usize = 24 * 1024;
const MAX_RECALLED_FACT_CHARS: usize = 2 * 1024;
const MAX_RECALL_TOTAL_CHARS: usize = 8 * 1024;
const MAX_HISTORY_MESSAGE_CHARS: usize = 16 * 1024;
const MAX_HISTORY_TOTAL_CHARS: usize = 64 * 1024;
const MAX_HISTORY_MESSAGES: usize = 6;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let truncated: String = value.chars().take(max_chars - 1).collect();
    format!("{truncated}…")
}

fn append_bounded_text(target: &mut String, value: &str, per_item: usize, remaining: &mut usize) {
    if *remaining == 0 {
        return;
    }
    let bounded = truncate_chars(value, per_item.min(*remaining));
    *remaining = (*remaining).saturating_sub(bounded.chars().count());
    target.push_str(&bounded);
}

/// Return only bounded conversational text. Tool blocks and historical system
/// prompts are deliberately excluded so a restored session cannot replay
/// transient prompt decorations or malformed tool-call sequences.
fn bounded_text_history(history: &[Message]) -> Vec<Message> {
    let mut remaining = MAX_HISTORY_TOTAL_CHARS;
    let mut selected = Vec::new();

    for message in history.iter().rev() {
        if selected.len() >= MAX_HISTORY_MESSAGES || remaining == 0 {
            break;
        }
        if !matches!(message.role, Role::User | Role::Assistant) {
            continue;
        }
        let text = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }
        let bounded = truncate_chars(&text, MAX_HISTORY_MESSAGE_CHARS.min(remaining));
        remaining = remaining.saturating_sub(bounded.chars().count());
        selected.push(match message.role {
            Role::User => Message::user(bounded),
            Role::Assistant => Message::assistant(bounded),
            Role::System => unreachable!(),
        });
    }

    selected.reverse();
    selected
}

fn build_request_messages(
    system_prompt: String,
    history: &[Message],
    effective_message: String,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(MAX_HISTORY_MESSAGES + 2);
    messages.push(Message::system(system_prompt));
    messages.extend(bounded_text_history(history));
    messages.push(Message::user(effective_message));
    messages
}

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
            &self.subsystems.runtime.lock().await.config().session_id,
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
            let prefix = self.subsystems.cached_prefix.lock().await;
            prefix.clone()
        };

        // Build effective user message with memory updates and SandboxFirst note.
        // Both go into the user turn to preserve cache-stable system prompt.
        // Memory updates are composed via compose_memory_block() — the same
        // pattern as ReActLoop::compose_user_message() / Controller::compose_user_message().
        let memory_block = self.compose_memory_block().await;

        // Reset Storm Breaker counters at the start of each turn so that
        // success/failure warnings don't accumulate across turns.
        {
            let mut sb = self.subsystems.storm_breaker.lock().await;
            sb.reset();
        }

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
            let loader = self.subsystems.skill_loader.lock().await;
            let skill_keywords: Vec<corpus::skill::keyword_matcher::SkillKeywords> = loader
                .plugins()
                .iter()
                .filter(|p| !p.keywords.is_empty())
                .map(|p| corpus::skill::keyword_matcher::SkillKeywords {
                    name: p.name.clone(),
                    keywords: p.keywords.clone(),
                    body: p.system_prompt.clone(),
                })
                .collect();
            drop(loader);
            let matched = corpus::skill::keyword_matcher::match_skills(message, &skill_keywords);
            let mut remaining = MAX_ACTIVATED_SKILLS_TOTAL_CHARS;
            for body in matched {
                if remaining == 0 {
                    break;
                }
                effective_message.push_str("\n<activated-skill>\n");
                append_bounded_text(
                    &mut effective_message,
                    &body,
                    MAX_ACTIVATED_SKILL_CHARS,
                    &mut remaining,
                );
                effective_message.push_str("\n</activated-skill>\n");
            }
        }

        // --- Fact recall from FactStore ---
        {
            let fs = self.subsystems.fact_store.lock().await;
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
                        let mut remaining = MAX_RECALL_TOTAL_CHARS;
                        for fact in &facts {
                            if remaining == 0 {
                                break;
                            }
                            recall_block.push_str("- ");
                            append_bounded_text(
                                &mut recall_block,
                                &fact.content,
                                MAX_RECALLED_FACT_CHARS,
                                &mut remaining,
                            );
                            recall_block.push_str(&format!(" (trust: {:.2})\n", fact.trust_score));
                            let _ = fs.record_feedback(fact.fact_id, true);
                        }
                        // Entity graph boost
                        let entities = FactStore::extract_entities(message);
                        for entity in entities.iter().take(3) {
                            if let Ok(eid) = fs.resolve_entity(entity) {
                                if let Ok(related) = fs.get_entity_facts(eid) {
                                    for rf in related.iter().take(1) {
                                        if !facts.iter().any(|f| f.fact_id == rf.fact_id) {
                                            if remaining == 0 {
                                                break;
                                            }
                                            recall_block.push_str("- ");
                                            append_bounded_text(
                                                &mut recall_block,
                                                &rf.content,
                                                MAX_RECALLED_FACT_CHARS,
                                                &mut remaining,
                                            );
                                            recall_block
                                                .push_str(&format!(" (entity: {})\n", entity));
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
            let cm = self.subsystems.core_memory.lock().await;
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
            let sr = self.subsystems.skill_router.lock().await;
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
            let fs = self.subsystems.fact_store.lock().await;
            let _ = fs.decay_stale();
        }

        // --- Configured pre_turn hook scripts ---
        if !self.subsystems.hooks_config.pre_turn.is_empty() {
            let hook_session_id = self.get_or_create_session(None).await.0;
            let hook_input = serde_json::json!({
                "prompt": message,
                "session_id": hook_session_id
            });
            let hook_outputs = self
                .run_hook_scripts(
                    &self.subsystems.hooks_config.pre_turn,
                    &hook_input.to_string(),
                )
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
            let hr = self.subsystems.hook_registry.lock().await;
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
            sm.push_user(message).await;
        }
        // Persist user message to recall memory
        {
            let (session_id, sm_arc) = self.get_or_create_session(None).await;
            let rm = self.subsystems.recall_memory.lock().await;
            let _ = rm.store(&session_id, "user_message", message, None);
            // Fire OnMemoryStore hook
            {
                let hr = self.subsystems.hook_registry.lock().await;
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
            let tools = self.subsystems.tools.lock().await;
            tools.definitions()
        };

        // Prepare execute_tool closure that runs tools through the guarded runner.
        let runner = self.subsystems.tool_runner.clone();
        let tools_arc = self.subsystems.tools.clone();
        let hook_registry_arc = self.subsystems.hook_registry.clone();
        let storm_breaker_arc = self.subsystems.storm_breaker.clone();
        let memory_queue_arc = self.subsystems.memory_queue.clone();
        let session_approvals_arc = self.subsystems.session_approvals.clone();
        let notify_tx_arc = self.notify_tx.clone();
        let debug_perf_arc = self.subsystems.debug_perf.clone();
        let self_field_arc = self.subsystems.self_field.clone();
        let working_dir = std::env::current_dir().unwrap_or_default();
        let (session_id, sm_arc) = self.get_or_create_session(None).await;
        let turn_count = sm_arc.lock().await.turn_count();
        drop(sm_arc);

        // RFC-014 recall injection: seed the Agora workspace for this turn.
        if let Err(e) = self
            .subsystems
            .agora
            .publish(&session_id, "turn_input", serde_json::json!(message))
            .await
        {
            tracing::warn!("agora publish (recall injection) failed: {e}");
        }

        // Clone self_field_arc before the execute_tool closure moves it,
        // so the tokio::spawn block below can also use it for per-turn Dasein injections.
        let self_field_arc_for_react = self_field_arc.clone();
        // Clone session_id before the execute_tool closure moves it, so the
        // Agora commit hook (turn end) can still reference it.
        let session_id_for_agora = session_id.clone();

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
                let exec_ctx = fabric::tool::ToolContext {
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
                        tool_result: Some(fabric::hook::HookToolResult {
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
        let approval_rx = self.subsystems.approval_rx.clone();
        let pending_approvals = self.subsystems.pending_approvals.clone();
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
            let sf = self.subsystems.self_field.lock().await;
            if let Some(ctx) = sf.dasein_prompt_injection() {
                format!("{}\n\n---\n\n{}", ctx, effective_message)
            } else {
                effective_message
            }
        };

        let config = self.subsystems.runtime.lock().await.config().clone();
        let llm_clone = llm.clone();
        let tool_defs_clone = tool_defs.clone();
        let goal_message = message.to_string();
        let goal_message_for_gw = goal_message.clone();

        // Pre-turn proactive compaction: compact the persisted history before
        // seeding the ReAct loop so the seed is already within token budget.
        // Persisted history contains the raw current user message. Remove it
        // from the historical tail and add the enriched form exactly once as
        // an ephemeral message below.
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

        let system_prompt_for_react = system_prompt.clone();
        let history_chars: usize = existing_messages
            .iter()
            .flat_map(|message| message.content.iter())
            .map(ContentBlock::estimate_chars)
            .sum();
        info!(
            system_chars = system_prompt.len(),
            history_chars,
            ephemeral_user_chars = effective_message.len(),
            "LLM request context assembled"
        );

        let mut react_task = tokio::spawn(async move {
            let harness_config = HarnessConfig {
                max_iterations: config.max_iterations,
                compaction_enabled: config.compaction_enabled,
                tail_token_budget: config.tail_token_budget,
                target_summary_chars: config.target_summary_chars,
                context_window_tokens: config.context_window_tokens,
                max_tool_calls: config.agent_loop.max_tool_calls,
                reflection_interval: config.agent_loop.reflection_interval,
                reflection_tool_call_limit: config.agent_loop.reflection_tool_call_limit,
                circuit_breaker_max_repeats: config.circuit_breaker.max_repeats,
                circuit_breaker_window_size: config.circuit_breaker.window_size,
                learning_enabled: config.learning_enabled,
            };
            let effective_tail = if config.tail_token_budget * 4 < config.context_window_tokens {
                config.context_window_tokens / 8
            } else {
                config.tail_token_budget
            };
            let compressor = Box::new(AdvancedCompressor::new(
                effective_tail,
                config.target_summary_chars,
                config.context_window_tokens,
            )) as Box<dyn cognit::harness::linear::CompactorTrait>;
            let mut react_loop = ReActLoop::new(harness_config, compressor);
            let request_messages = build_request_messages(
                system_prompt_for_react,
                &existing_messages,
                effective_message,
            );
            react_loop.seed_messages(request_messages);
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
                    &DynLlmRef(&*llm_clone),
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

        // Accumulated token usage across this turn
        let mut acc_tokens_in: u64 = 0;
        let mut acc_tokens_out: u64 = 0;

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
                            if let Err(e) = self
                                .subsystems
                                .agora
                                .trace(
                                    &session_id_for_agora,
                                    "tool_result",
                                    serde_json::json!({
                                        "call_id": call_id,
                                        "content": result.content,
                                        "is_error": result.is_error,
                                    }),
                                )
                                .await
                            {
                                tracing::warn!(target: "agora", error = %e, "agora trace append failed");
                            }
                        }
                        Event::Usage {
                            tokens_in,
                            tokens_out,
                            ..
                        } => {
                            acc_tokens_in += *tokens_in as u64;
                            acc_tokens_out += *tokens_out as u64;
                        }
                        _ => {}
                    }

                    if let Some(client_event) = event_to_client_event(&event) {
                        if let Ok(json_str) = event_to_json(&client_event) {
                            if let Some(ref tx) = notify_tx {
                                let _ = tx.send(json_str).await;
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
            if let Some(client_event) = event_to_client_event(&event) {
                if let Ok(json_str) = event_to_json(&client_event) {
                    if let Some(ref tx) = notify_tx {
                        let _ = tx.send(json_str).await;
                    }
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

        // Update SessionGateway state with current turn metrics for external debug access.
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

        // Coordinate: quick mood update after the turn (Task 23)
        if turn_succeeded {
            self.coordinate(&turn_count, &text).await;
        }

        // Record turn in perf counter (token counts from accumulated Usage events).
        self.subsystems
            .debug_perf
            .record_turn(acc_tokens_in, acc_tokens_out);

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
            let hr = self.subsystems.hook_registry.lock().await;
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
        if turn_succeeded {
            let mut am = self.subsystems.auto_memory.lock().await;
            if let Ok(facts) = am.analyze_and_store(message, &text).await {
                if !facts.is_empty() {
                    info!(count = facts.len(), "Auto-memory: stored facts");
                }
            }
        }

        // Push tool call messages and assistant response to session manager
        // This ensures the session history has the correct structure for OpenAI API
        let turn = if turn_succeeded {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let mut sm = sm_arc.lock().await;

            // Push tool call messages if any
            if !tool_calls_for_session.is_empty() {
                use fabric::message::{ContentBlock, Message, Role};

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
                })
                .await;

                // Push tool result messages — ALL in ONE combined user message.
                // Anthropic API requires all tool_result blocks for a given
                // assistant(tool_use) message to be in a single subsequent message.
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
            let tc = sm_arc.lock().await.turn_count();
            tc
        };
        // Persist assistant response to recall memory
        if turn_succeeded {
            let session_id = self.get_or_create_session(None).await.0;
            let rm = self.subsystems.recall_memory.lock().await;
            let _ = rm.store(&session_id, "assistant_message", &text, None);
            // Fire OnMemoryStore hook
            {
                let hr = self.subsystems.hook_registry.lock().await;
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
        let entry = self.subsystems.reflector.reflect_conversation(
            &task_summary,
            ReflectionTrigger::TaskComplete,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        // Store reflection — drop lock guard before re-locking for evolution check
        let store_result = {
            let mem = self.subsystems.episodic_memory.lock().await;
            mem.store_reflection(&entry)
        };
        if let Err(e) = store_result {
            warn!(error = %e, "Failed to store chat reflection");
        } else {
            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");

            // Periodic evolution trigger: every 10 reflections, run ExperienceSummarizer
            let mem = self.subsystems.episodic_memory.lock().await;
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
            if let Err(e) = self
                .subsystems
                .runtime
                .lock()
                .await
                .post_evolution(
                    &task_summary,
                    &text,
                    success,
                    metrics.tool_calls_made,
                    metrics.tool_errors,
                    metrics.elapsed_ms,
                    metrics.iterations,
                    &*self.subsystems.pipeline,
                )
                .await
            {
                warn!(error = %e, "post_evolution failed");
            }
        }

        // RFC-014 commit: snapshot the Agora workspace at turn end, then
        // persist it to recall memory (RFC-018 Phase 1 — best-effort, never
        // propagated into the chat turn).
        match self.subsystems.agora.snapshot(&session_id_for_agora).await {
            Ok(snap) => {
                tracing::debug!(target: "agora", "workspace snapshot: {snap}");
                let rm = self.subsystems.recall_memory.lock().await;
                if let Err(e) = rm.store(
                    &session_id_for_agora,
                    "agora_snapshot",
                    &snap.to_string(),
                    None,
                ) {
                    tracing::warn!(target: "agora", error = %e, "agora snapshot persist failed");
                }
            }
            Err(e) => tracing::warn!("agora snapshot failed: {e}"),
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "response": text, "turn": turn }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(message: &Message) -> &str {
        match &message.content[0] {
            ContentBlock::Text { text } => text,
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn bounded_history_excludes_system_and_tool_blocks() {
        let history = vec![
            Message::system("large transient prefix"),
            Message::user("raw user"),
            Message::tool_result("call-1", "tool output", false),
            Message::assistant("raw assistant"),
        ];

        let bounded = bounded_text_history(&history);

        assert_eq!(bounded.len(), 2);
        assert_eq!(text_of(&bounded[0]), "raw user");
        assert_eq!(text_of(&bounded[1]), "raw assistant");
    }

    #[test]
    fn bounded_history_caps_restored_injected_payloads() {
        let huge = format!("<activated-skill>{}</activated-skill>", "x".repeat(200_000));
        let history = vec![Message::user(huge)];

        let bounded = bounded_text_history(&history);

        assert_eq!(bounded.len(), 1);
        assert!(text_of(&bounded[0]).chars().count() <= MAX_HISTORY_MESSAGE_CHARS);
    }

    #[test]
    fn bounded_text_is_utf8_safe_and_respects_budget() {
        let mut output = String::new();
        let mut remaining = 8;

        append_bounded_text(&mut output, "机器人上下文非常长", 6, &mut remaining);

        assert!(output.is_char_boundary(output.len()));
        assert!(output.chars().count() <= 6);
        assert!(remaining <= 2);
    }

    #[test]
    fn request_contains_one_system_prefix_and_one_ephemeral_user_message() {
        let history = vec![
            Message::system("old prefix that must not be replayed"),
            Message::user("raw prior user"),
            Message::assistant("raw prior assistant"),
        ];

        let messages = build_request_messages(
            "current prefix".into(),
            &history,
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user".into(),
        );

        assert_eq!(
            messages.iter().filter(|m| m.role == Role::System).count(),
            1
        );
        assert_eq!(text_of(&messages[0]), "current prefix");
        assert_eq!(
            text_of(messages.last().unwrap()),
            "<activated-skill>ephemeral</activated-skill>\ncurrent raw user"
        );
        assert!(!messages.iter().any(|m| text_of(m).contains("old prefix")));
    }
}
