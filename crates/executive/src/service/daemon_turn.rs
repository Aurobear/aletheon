//! Daemon Turn Orchestrator — the macro-kernel turn pipeline for daemon chat.
//!
//! This module extracts the orchestration logic from `RequestHandler::handle_chat`
//! into the service layer. The handler becomes a thin delegation layer that:
//!
//! 1. Parses the JSON-RPC request
//! 2. Delegates to `DaemonTurnOrchestrator::execute_turn()`
//! 3. Formats the JSON-RPC response
//!
//! # Kernel primitives wired
//!
//! - **ProcessTable**: main agent process is registered and tracked.
//! - **OperationTable**: each turn creates an operation for cancellation tracking.
//! - **SupervisorTree**: agent process has a restart policy.
//! - **AdmissionController**: tool execution gates through admission (production).
//! - **MailboxService**: registered per agent process for future inter-process comms.

use crate::core::core_systems::CoreSystems;
use crate::core::session_gateway::SessionGateway;
use crate::kernel::chronos::SystemClock;
use crate::kernel::operation::OperationTable;
use crate::kernel::process::ProcessTable;
use crate::kernel::supervision::{RestartPolicy, SupervisorTree};
use crate::r#impl::daemon::handler::tool_executor::TurnToolExecutor;
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::r#impl::daemon::session_manager::SessionManager;
use cognit::harness::event_sink::{ChannelEventSink, Event};
use cognit::harness::linear::TurnMetrics;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::{
    AdmissionController, AdmissionRequest, AgentId, CapabilityId, CapabilityScope, Clock,
    ContentBlock, Context as AbiContext, Intent, IntentSource, LlmProvider, Message, NamespaceId,
    OperationKind, OperationManager, OperationRequest, PrincipalId, ProcessId, ProcessManager,
    ProcessSignal, ReflectionTrigger, Role, SandboxDecision, SandboxRequirement,
    SelfFieldOps, SpawnSpec, TurnRequest, UsageReport, Verdict,
};
use fabric::types::admission::RiskLevel;
use mnemosyne::FactStore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::daemon_react::{submit_streaming_daemon_turn, DaemonStreamingTurnContext};

// Re-import constants and helpers from chat.rs
const MAX_ACTIVATED_SKILL_CHARS: usize = 12 * 1024;
const MAX_ACTIVATED_SKILLS_TOTAL_CHARS: usize = 24 * 1024;
const MAX_RECALLED_FACT_CHARS: usize = 2 * 1024;
const MAX_RECALL_TOTAL_CHARS: usize = 8 * 1024;
const MAX_HISTORY_MESSAGE_CHARS: usize = 16 * 1024;
const MAX_HISTORY_TOTAL_CHARS: usize = 64 * 1024;
const MAX_HISTORY_MESSAGES: usize = 6;

// ---------------------------------------------------------------------------
// Text helpers (same as chat.rs)
// ---------------------------------------------------------------------------

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

fn bounded_text_history(history: &[Message]) -> Vec<Message> {
    let mut bounded: Vec<Message> = Vec::new();
    let mut remaining = MAX_HISTORY_TOTAL_CHARS;
    for message in history.iter().rev().take(MAX_HISTORY_MESSAGES) {
        if remaining == 0 {
            break;
        }
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    let bounded_text =
                        truncate_chars(text, MAX_HISTORY_MESSAGE_CHARS.min(remaining));
                    remaining = remaining.saturating_sub(bounded_text.chars().count());
                    content_blocks.push(ContentBlock::Text { text: bounded_text });
                }
                // Skip non-text blocks (tool use/results are replayed by the harness)
                _ => {}
            }
        }
        if !content_blocks.is_empty() {
            bounded.push(Message {
                role: message.role,
                content: content_blocks,
            });
        }
    }
    bounded.reverse();
    bounded
}

fn build_request_messages(
    system_prompt: String,
    history: &[Message],
    effective_user_message: String,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(Message::system(system_prompt));
    messages.extend_from_slice(history);
    messages.push(Message::user(effective_user_message));
    messages
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Bundled state for executing a daemon chat turn through the kernel pipeline.
///
/// Created once per daemon instance (or per `RequestHandler`) and reused across
/// turns. The process table tracks the main agent across its entire lifecycle;
/// per-turn operations are created in `execute_turn()`.
pub struct DaemonTurnOrchestrator {
    // --- Kernel primitives ---
    process_table: Arc<ProcessTable>,
    operation_table: Arc<OperationTable>,
    supervisor: Mutex<SupervisorTree>,
    clock: Arc<dyn Clock>,
    admission: Arc<dyn AdmissionController>,
    mailbox_service: Arc<InProcessMailboxService>,
    main_agent_process_id: Mutex<Option<ProcessId>>,

    // --- Subsystem handles (mirrors RequestHandler fields) ---
    subsystems: Arc<CoreSystems>,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    session_gateway: Arc<SessionGateway>,
    llm: Arc<dyn LlmProvider>,
    model_router: Arc<ModelRouter>,
    notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    active_connections: Arc<AtomicUsize>,
    started_at: Instant,
    daemon_cancel_token: Option<CancellationToken>,
}

impl DaemonTurnOrchestrator {
    /// Create the orchestrator, wiring all kernel primitives.
    pub fn new(
        subsystems: Arc<CoreSystems>,
        sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
        session_gateway: Arc<SessionGateway>,
        llm: Arc<dyn LlmProvider>,
        model_router: Arc<ModelRouter>,
        notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
        active_connections: Arc<AtomicUsize>,
        started_at: Instant,
        daemon_cancel_token: Option<CancellationToken>,
    ) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let process_table = Arc::new(ProcessTable::new(clock.clone()));
        let operation_table = Arc::new(OperationTable::new(clock.clone()));
        let supervisor = Mutex::new(SupervisorTree::new());
        let admission = subsystems.admission.clone();
        let mailbox_service = Arc::new(InProcessMailboxService::new());

        Self {
            process_table,
            operation_table,
            supervisor,
            clock,
            admission,
            mailbox_service,
            main_agent_process_id: Mutex::new(None),
            subsystems,
            sessions,
            session_gateway,
            llm,
            model_router,
            notify_tx,
            active_connections,
            started_at,
            daemon_cancel_token,
        }
    }

    /// Access the shared notify_tx for external updates (e.g. set_notify_channel).
    pub fn notify_tx(&self) -> &Arc<Mutex<Option<mpsc::Sender<String>>>> {
        &self.notify_tx
    }

    // ------------------------------------------------------------------
    // Kernel process management
    // ------------------------------------------------------------------

    /// Ensure the main daemon agent is registered in the process table.
    async fn ensure_main_agent(&self) -> anyhow::Result<ProcessId> {
        let mut guard = self.main_agent_process_id.lock().await;
        if let Some(pid) = *guard {
            return Ok(pid);
        }
        let handle = self
            .process_table
            .spawn(SpawnSpec {
                agent_id: AgentId::new(),
                namespace: NamespaceId("daemon".into()),
                initial_operation: Some(OperationKind::SubAgent),
                ..SpawnSpec::default()
            })
            .await?;
        self.process_table
            .signal(handle.id, ProcessSignal::Start)
            .await?;
        self.supervisor.lock().await.supervise(
            handle.id,
            RestartPolicy::RestartOnFailure { max_restarts: 3 },
        );
        *guard = Some(handle.id);
        info!(process_id = ?handle.id, "Main daemon agent registered in process table");
        Ok(handle.id)
    }

    // ------------------------------------------------------------------
    // Session helpers (mirror RequestHandler)
    // ------------------------------------------------------------------

    async fn get_or_create_session(
        &self,
        session_id: Option<&str>,
    ) -> (String, Arc<Mutex<SessionManager>>) {
        let id = if let Some(sid) = session_id {
            sid.to_string()
        } else {
            self.subsystems.default_session_id.lock().await.clone()
        };
        {
            let sessions = self.sessions.lock().await;
            if let Some(sm) = sessions.get(&id) {
                return (id, sm.clone());
            }
        }
        match SessionManager::new(
            &self.subsystems.data_dir,
            id.clone(),
            self.subsystems.context_window,
        )
        .await
        {
            Ok(sm) => {
                let sm_arc = Arc::new(Mutex::new(sm));
                self.sessions
                    .lock()
                    .await
                    .insert(id.clone(), sm_arc.clone());
                self.subsystems
                    .session_created_at
                    .lock()
                    .await
                    .insert(id.clone(), Instant::now());
                info!(session_id = %id, "Session created on demand");
                (id, sm_arc)
            }
            Err(e) => {
                warn!(error = %e, session_id = %id, "Failed to create session on demand, using default");
                let default_id = self.subsystems.default_session_id.lock().await.clone();
                let sessions = self.sessions.lock().await;
                let sm = sessions.get(&default_id).cloned().unwrap();
                (default_id, sm)
            }
        }
    }

    async fn begin_turn_token(&self) -> CancellationToken {
        let ct = CancellationToken::new();
        let mut token = self.subsystems.cancel_token.lock().await;
        *token = Some(ct.clone());
        ct
    }

    // ------------------------------------------------------------------
    // SelfField helpers
    // ------------------------------------------------------------------

    async fn sf_review(&self, intent: &Intent, ctx: &AbiContext) -> anyhow::Result<Verdict> {
        let sf = self.subsystems.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    async fn sf_narrate(&self, event: &str, reason: &str) {
        let sf = self.subsystems.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            let _mood = dasein.quick_mood_update(turn_text);
            tracing::info!(turn = turn, "Dasein mood updated via coordinator");
        }
    }

    async fn compose_memory_block(&self) -> String {
        let mut queue = self.subsystems.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);
        let items: Vec<String> = updates.iter().map(|m| format!("- {}", m)).collect();
        format!("<memory-update>\n{}\n</memory-update>", items.join("\n"))
    }

    // ------------------------------------------------------------------
    // Pre-turn injection phases
    // ------------------------------------------------------------------

    async fn inject_keyword_skills(&self, message: &str, effective_message: &mut String) {
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
                effective_message,
                &body,
                MAX_ACTIVATED_SKILL_CHARS,
                &mut remaining,
            );
            effective_message.push_str("\n</activated-skill>\n");
        }
    }

    async fn inject_fact_recall(&self, message: &str, effective_message: &mut String) {
        let fs = self.subsystems.fact_store.lock().await;
        let keywords: Vec<String> = message
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_lowercase())
            .collect();
        let query = keywords.join(" ");
        if query.len() < 8 {
            return;
        }
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
                let entities = FactStore::extract_entities(message);
                for entity in entities.iter().take(3) {
                    let _ = fs.resolve_entity(entity).map(|eid| {
                        let _ = fs.get_entity_facts(eid).map(|related| {
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
                                    recall_block.push_str(&format!(" (entity: {})\n", entity));
                                }
                            }
                        });
                    });
                }
                info!(count = facts.len(), "Fact recall injected");
                effective_message.push_str(&recall_block);
            }
        }
    }

    async fn inject_core_memory(&self, effective_message: &mut String) {
        let cm = self.subsystems.core_memory.lock().await;
        let mut core_lines = Vec::new();
        for (label, block) in cm.blocks() {
            if block.read_only || block.value.is_empty() {
                continue;
            }
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

    async fn inject_skill_suggestion(&self, message: &str, effective_message: &mut String) {
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

    async fn decay_stale_facts(&self) {
        let fs = self.subsystems.fact_store.lock().await;
        let _ = fs.decay_stale();
    }

    // ------------------------------------------------------------------
    // Post-turn phases
    // ------------------------------------------------------------------

    async fn run_post_turn_hooks(&self) {
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

    async fn extract_auto_memory(&self, message: &str, text: &str) {
        let mut am = self.subsystems.auto_memory.lock().await;
        if let Ok(facts) = am.analyze_and_store(message, text).await {
            if !facts.is_empty() {
                info!(count = facts.len(), "Auto-memory: stored facts");
            }
        }
    }

    async fn record_turn_reflection(&self, task_summary: &str, text: &str, turn: usize) {
        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();
        let resp_len = text.len();
        if resp_len > 500 {
            what_worked.push(format!("Detailed response ({} chars)", resp_len));
        } else if resp_len > 100 {
            what_worked.push(format!("Concise response ({} chars)", resp_len));
        } else {
            what_worked.push(format!("Brief response ({} chars)", resp_len));
        }
        let text_lower = text.to_lowercase();
        for indicator in &[
            "error",
            "failed",
            "unable",
            "cannot",
            "couldn't",
            "sorry, i",
            "i don't know",
        ] {
            if text_lower.contains(indicator) {
                what_failed.push(format!("Response contains '{}'", indicator));
            }
        }
        for indicator in &[
            "i learned",
            "i now understand",
            "i realize",
            "correction:",
            "actually,",
        ] {
            if text_lower.contains(indicator) {
                learned.push(format!("Self-correction detected: '{}'", indicator));
            }
        }
        what_worked.push(format!("Conversation turn #{}", turn));
        let has_failures = !what_failed.is_empty();
        let entry = self.subsystems.reflector.reflect_conversation(
            task_summary,
            ReflectionTrigger::TaskComplete,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        let store_result = {
            let mem = self.subsystems.episodic_memory.lock().await;
            mem.store_reflection(&entry)
        };
        if let Err(e) = store_result {
            warn!(error = %e, "Failed to store chat reflection");
        } else {
            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");
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
    }

    async fn run_post_evolution(&self, task_summary: &str, text: &str, metrics: &TurnMetrics) {
        let success = metrics.completed_normally && !text.starts_with("error:");
        if let Err(e) = self
            .subsystems
            .runtime
            .lock()
            .await
            .post_evolution(
                task_summary,
                text,
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

    async fn commit_agora_snapshot(&self, session: &str) {
        match self.subsystems.agora.snapshot(session).await {
            Ok(snap) => {
                tracing::debug!(target: "agora", "workspace snapshot: {snap}");
                let rm = self.subsystems.recall_memory.lock().await;
                if let Err(e) = rm.store(session, "agora_snapshot", &snap.to_string(), None) {
                    tracing::warn!(target: "agora", error = %e, "agora snapshot persist failed");
                }
            }
            Err(e) => tracing::warn!("agora snapshot failed: {e}"),
        }
    }

    // ==================================================================
    // execute_turn — the main orchestration entry point
    // ==================================================================

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
        let sf_ctx = AbiContext::new(&session_id, std::env::current_dir().unwrap_or_default());
        let verdict = self.sf_review(&intent, &sf_ctx).await;

        // Sandbox requirement from SelfField verdict — passed to tool admission.
        let mut sandbox_requirement = SandboxRequirement::NotRequired;

        match verdict {
            Ok(Verdict::Deny { ref reason }) => {
                warn!(reason = %reason, "SelfField denied chat intent");
                self.sf_narrate("chat_denied", reason).await;
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32010, "message": format!("Intent denied by SelfField: {}", reason)}});
            }
            Ok(Verdict::SandboxFirst { ref reason }) => {
                warn!(reason = %reason, "SelfField requires sandbox; tools will be gated through admission");
                self.sf_narrate("chat_sandbox_required", reason).await;
                // Route through admission gate instead of failing here.
                // Each tool call builds an AdmissionRequest with
                // SandboxRequirement::Required, and ProductionAdmissionController
                // returns SandboxDecision::Required (fail-closed).
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
            // run_hook_scripts via inline logic
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

        // Agora seed
        if let Err(e) = self
            .subsystems
            .agora
            .publish(&sess_id, "turn_input", serde_json::json!(message))
            .await
        {
            tracing::warn!("agora publish (recall injection) failed: {e}");
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
                    input_summary: format!("{:?}", &inp).chars().take(200).collect(),
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
                        format!("Sandbox required but execution infrastructure not available for '{n}'"),
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
                            if let Err(e) = self.subsystems.agora.record_evidence(&session_id_for_agora, &evidence).await {
                                tracing::warn!(target: "agora", error = %e, "agora evidence trace append failed");
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
        self.commit_agora_snapshot(&session_id_for_agora).await;

        // -- Kernel: mark turn operation completed --
        let _ = self.operation_table.succeed(operation.id).await;

        json!({"jsonrpc": "2.0", "id": id, "result": {"response": text, "turn": turn}})
    }
}
