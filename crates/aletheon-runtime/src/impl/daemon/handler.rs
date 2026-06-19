use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::session_manager::SessionManager;
use crate::core::config::RuntimeConfig;
use crate::core::orchestrator::AletheonRuntime;
use crate::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};
use crate::r#impl::orchestration::builtin::{CodeAgent, FsAgent, NetAgent};
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::session::store::SessionStore;
use crate::CoreMemory;
use crate::ProviderRegistry;
use crate::RecallMemory;
use aletheon_abi::envelope::*;
use aletheon_abi::hook::{HookContext, HookPoint, HookResult};
use aletheon_abi::Registry;
use aletheon_abi::{
    Context as AbiContext, Intent, IntentSource, ReflectionTrigger, SelfFieldOps, Subsystem,
    SubsystemContext, Verdict,
};
use aletheon_body::r#impl::sandbox::executor::{SandboxExecutor, SandboxPreference};
use aletheon_body::r#impl::security::approval::ApprovalDecision;
use aletheon_body::r#impl::security::audit::AuditLogger;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::security::socket_approval::{PendingApproval, SocketApprovalGate};
use aletheon_body::r#impl::tools::Tool;
use aletheon_body::r#impl::tools::ToolRegistry;
use aletheon_brain::core::reflector::Reflector;
use aletheon_brain::core::ExperienceSummarizer;
use aletheon_brain::r#impl::llm::LlmProvider;
use aletheon_comm::envelope::Payload;
use aletheon_comm::CommunicationBus;
use aletheon_memory::episodic::EpisodicMemory;
use aletheon_meta::r#impl::meta_runtime::self_reader::SelfReader;
use aletheon_self::r#impl::perception::bridge::PerceptionInjection;
use aletheon_self::{SelfField, SelfFieldConfig};
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};

use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};
use crate::r#impl::hooks::builtin::audit_hook;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::skills::loader::SkillLoader;
use crate::r#impl::skills::plugin::register_skill;

use super::prefix_builder::PrefixBuilder;
use super::DaemonConfig;

/// Session state wrapping the new AletheonRuntime.
///
/// NOTE: The old Engine god-object has been replaced by AletheonRuntime.
/// Methods like `run_turn`, `messages`, `set_perception_rx` etc. no longer
/// exist on the runtime.  This handler exposes a thin shim that delegates
/// to `AletheonRuntime::process()`.  A full migration of the daemon to the
/// new intent/plan/execute pipeline is tracked separately.
struct SessionState {
    runtime: AletheonRuntime,
    /// Pending input waiting to be processed via the cognitive pipeline.
    pending_input: Option<String>,
}

#[derive(Clone)]
pub struct RequestHandler {
    state: Arc<Mutex<SessionState>>,
    llm: Arc<dyn LlmProvider>,
    session_manager: Arc<Mutex<SessionManager>>,
    recall_memory: Arc<Mutex<RecallMemory>>,
    data_dir: PathBuf,
    /// Retained for future use; currently unused after Engine removal.
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
    reflector: Reflector,
    episodic_memory: Arc<Mutex<EpisodicMemory>>,
    /// SelfField — the policy engine that provides identity, cares, and boundary data.
    self_field: Arc<Mutex<SelfField>>,
    /// Loads skill markdown files from the skills directory and caches them.
    skill_loader: Arc<Mutex<SkillLoader>>,
    /// Cache-stable system prompt prefix, built once at boot.
    /// Same inputs = same bytes = cache hit on DeepSeek/Mimo.
    cached_prefix: Arc<Mutex<String>>,
    /// Queue for memory updates that arrive mid-session.
    /// Drained into user turns as `<memory-update>` XML blocks
    /// so the system prompt prefix stays byte-stable.
    memory_queue: Arc<Mutex<Vec<String>>>,
    /// The base system prompt from config, retained for prefix rebuilds.
    config_prompt: String,
    /// Core memory reference, retained for prefix rebuilds on skill reload.
    core_memory: Arc<Mutex<CoreMemory>>,
    /// Lifecycle hook registry.
    hook_registry: Arc<Mutex<HookRegistry>>,
    /// CommunicationBus for inter-module communication.
    /// When `Some`, SelfField review/narrate calls go through the bus.
    bus: Option<Arc<CommunicationBus>>,
    /// Guarded tool runner (policy -> approval -> loop detector -> sandbox -> audit).
    /// Wired to the SocketApprovalGate so L2+ requests are forwarded to the client.
    tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    /// Tool registry shared with BodyModule; kept here for ReAct loop tool execution.
    tools: Arc<Mutex<ToolRegistry>>,
    /// Receiver for pending approval requests from the SocketApprovalGate.
    /// Drained during chat turns to relay approval requests to the client.
    approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    /// Map from approval_id to the oneshot sender that resolves the pending approval.
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    /// Channel to send out-of-band JSON-RPC notifications to the connected client.
    /// Used to push `approval_request` notifications during a chat turn.
    notify_tx: Option<mpsc::Sender<String>>,
}

impl RequestHandler {
    /// Set the notification channel for out-of-band messages to the client.
    /// Returns the receiver end that the server should drain alongside responses.
    pub fn set_notify_channel(&mut self, tx: mpsc::Sender<String>) {
        self.notify_tx = Some(tx);
    }

    /// Create a notification channel and wire it to the handler.
    /// Returns the receiver for the server to consume out-of-band notifications.
    pub fn create_notify_channel(&mut self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(64);
        self.notify_tx = Some(tx);
        rx
    }

    pub async fn new(
        config: &DaemonConfig,
        registry: &ProviderRegistry,
        _perception_rx: mpsc::Receiver<PerceptionInjection>,
    ) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create("")?);
        info!(provider = llm.name(), "LLM provider initialized");

        // Create session and journal
        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let session_store = SessionStore::new(&data_dir)?;
        session_store.create_session(&session_id)?;

        info!(session_id = %session_id, "Created new session");

        // Create memory instances
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db_path = data_dir.join("recall_memory.db");
        let recall_memory = Arc::new(Mutex::new(RecallMemory::new(&recall_db_path)?));

        // Create SessionManager (owns the journal, history, and compaction)
        let session_manager = SessionManager::new(
            &data_dir,
            session_id.clone(),
            100_000, // max_tokens: ~100k default context window
        )
        .await?;

        // Register tools including memory tools
        let mut tools = ToolRegistry::default();
        tools.register(Arc::new(CoreMemoryAppendTool {
            memory: core_memory.clone(),
        }));
        tools.register(Arc::new(CoreMemoryReplaceTool {
            memory: core_memory.clone(),
        }));
        tools.register(Arc::new(MemorySearchTool {
            recall: recall_memory.clone(),
        }));

        // Create security components
        let sandbox_pref = SandboxPreference::from_str(&config.sandbox_preference);
        let sandbox = SandboxExecutor::new(sandbox_pref);
        let audit_path = data_dir.join("audit.jsonl");
        let audit_logger = AuditLogger::new(audit_path)?;

        // Install SocketApprovalGate — forwards L2+ approval requests to the
        // connected client via out-of-band JSON-RPC notifications.
        let (approval_gate, approval_rx) = SocketApprovalGate::new();
        let tool_runner = Arc::new(Mutex::new(
            ToolRunnerWithGuard::new(sandbox, audit_logger)
                .with_approval_gate(Arc::new(approval_gate)),
        ));

        let runtime_config = RuntimeConfig {
            session_id: session_id.clone(),
            ..Default::default()
        };

        // Create agent registry: try config-based loading first, fall back to builtins
        let agents_dir = PathBuf::from("agents");

        // Collect default tools for config-based agent loading
        let default_tools: Vec<Box<dyn Tool>> = vec![
            Box::new(aletheon_body::r#impl::tools::file_read::FileReadTool),
            Box::new(aletheon_body::r#impl::tools::file_write::FileWriteTool),
            Box::new(aletheon_body::r#impl::tools::bash_exec::BashExecTool),
            Box::new(aletheon_body::r#impl::tools::system_status::SystemStatusTool),
            Box::new(aletheon_body::r#impl::tools::process_list::ProcessListTool),
        ];

        // Each config-loaded agent needs its own LLM instance; factory creates fresh ones
        let llm_factory = || registry.resolve_and_create("");
        let agent_registry = Arc::new(
            AgentRegistry::load_from_config(&agents_dir, &default_tools, &llm_factory).await,
        );

        // Register built-in agents as fallbacks if no config agents were loaded
        if agent_registry.count().await == 0 {
            info!("No config agents found, registering built-in agents");
            agent_registry
                .register(Arc::new(tokio::sync::RwLock::new(FsAgent::new(
                    registry.resolve_and_create("")?,
                ))))
                .await;
            agent_registry
                .register(Arc::new(tokio::sync::RwLock::new(NetAgent::new(
                    registry.resolve_and_create("")?,
                ))))
                .await;
            agent_registry
                .register(Arc::new(tokio::sync::RwLock::new(CodeAgent::new(
                    registry.resolve_and_create("")?,
                ))))
                .await;
        }

        let runtime = AletheonRuntime::new(runtime_config);

        // Create reflector and episodic memory for post-chat reflection
        let reflector = Reflector::new();
        let episodic_db_path = data_dir.join("episodic.db");
        let mut episodic_memory = EpisodicMemory::new(episodic_db_path);
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: data_dir.clone(),
            config: serde_json::Value::Null,
        };
        episodic_memory.init(&ctx).await?;
        let episodic_memory = Arc::new(Mutex::new(episodic_memory));

        // Create SelfField for genome reads and policy engine
        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            ..Default::default()
        };
        let self_field = Arc::new(Mutex::new(SelfField::new(self_field_config)));

        // Initialize skill loader from the default skills directory
        let skills_dir = aletheon_abi::paths::skills_dir();
        let mut skill_loader = SkillLoader::new(skills_dir);
        let loaded = skill_loader.load_all_enhanced();
        if loaded > 0 {
            info!(count = loaded, "Skills loaded at startup");
        }

        // Create hook registry and register builtin hooks
        let mut hook_registry = HookRegistry::new();
        audit_hook::register_audit_hook(&mut hook_registry);

        // Register skill hooks from loaded plugins
        for plugin in skill_loader.plugins() {
            register_skill(plugin, &mut tools, &mut hook_registry);
        }
        let hook_registry = Arc::new(Mutex::new(hook_registry));

        // Build the cache-stable prefix once at boot.
        // Same inputs = same bytes = cache hit on DeepSeek/Mimo.
        let cm = core_memory.lock().await;
        let cached_prefix = PrefixBuilder::build(&config.system_prompt, skill_loader.skills(), &cm);
        drop(cm);
        info!(len = cached_prefix.len(), "Cache-stable prefix built");

        // Create CommunicationBus and spawn module handlers
        let bus = Arc::new(CommunicationBus::new());

        // Spawn SelfFieldModule handler — shares the same SelfField instance
        {
            let sf_module = crate::r#impl::engine::modules::self_field_module::SelfFieldModule::new(
                self_field.clone(),
            );
            let bus_clone = bus.clone();
            tokio::spawn(async move {
                sf_module.run(bus_clone).await;
            });
        }

        // Spawn MemoryModule handler — shares the same CoreMemory and RecallMemory instances
        {
            let mem_module = crate::r#impl::engine::modules::memory_module::MemoryModule::new(
                core_memory.clone(),
                Some(recall_memory.clone()),
            );
            let bus_clone = bus.clone();
            tokio::spawn(async move {
                mem_module.run(bus_clone).await;
            });
        }

        // Spawn BodyModule handler — shares the same ToolRegistry instance
        let tools = Arc::new(Mutex::new(tools));
        {
            let body_module =
                crate::r#impl::engine::modules::body_module::BodyModule::new(tools.clone());
            let bus_clone = bus.clone();
            tokio::spawn(async move {
                body_module.run(bus_clone).await;
            });
        }

        info!("CommunicationBus created with SelfField, Memory, and Body module handlers");

        Ok(Self {
            state: Arc::new(Mutex::new(SessionState {
                runtime,
                pending_input: None,
            })),
            llm,
            session_manager: Arc::new(Mutex::new(session_manager)),
            recall_memory,
            data_dir,
            agent_registry,
            reflector,
            episodic_memory,
            self_field,
            skill_loader: Arc::new(Mutex::new(skill_loader)),
            cached_prefix: Arc::new(Mutex::new(cached_prefix)),
            memory_queue: Arc::new(Mutex::new(Vec::new())),
            config_prompt: config.system_prompt.clone(),
            core_memory,
            hook_registry,
            bus: Some(bus),
            tool_runner,
            tools,
            approval_rx: Arc::new(Mutex::new(approval_rx)),
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            notify_tx: None,
        })
    }

    /// Review an intent through SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    async fn sf_review(&self, intent: &Intent, ctx: &AbiContext) -> anyhow::Result<Verdict> {
        if let Some(ref bus) = self.bus {
            let req = SelfFieldRequest::Review {
                intent: intent.clone(),
                ctx: serde_json::to_value(ctx).unwrap_or_default(),
            };
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::SelfField),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<SelfFieldResponse>(val.clone()) {
                            Ok(SelfFieldResponse::Verdict { verdict }) => return Ok(verdict),
                            Ok(SelfFieldResponse::Error { message }) => {
                                return Err(anyhow::anyhow!("SelfField review error: {}", message));
                            }
                            Ok(other) => {
                                return Err(anyhow::anyhow!(
                                    "Unexpected SelfField response: {:?}",
                                    other
                                ));
                            }
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Failed to deserialize SelfFieldResponse: {}",
                                    e
                                ));
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for SelfField review failed, falling back to direct");
                }
            }
        }
        // Fallback: direct lock
        let sf = self.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    /// Record a narrative entry in SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    async fn sf_narrate(&self, event: &str, reason: &str) {
        if let Some(ref bus) = self.bus {
            let req = SelfFieldRequest::Narrate {
                event: event.to_string(),
                reason: reason.to_string(),
            };
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::SelfField),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<SelfFieldResponse>(val.clone()) {
                            Ok(SelfFieldResponse::Narrated) => return,
                            Ok(SelfFieldResponse::Error { message }) => {
                                warn!(error = %message, "SelfField narrate error via bus");
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for SelfField narrate failed, falling back to direct");
                }
            }
        }
        // Fallback: direct lock
        let sf = self.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    /// Drain all queued memory updates into a `<memory-update>` XML block.
    /// Returns empty string if the queue is empty.
    /// This keeps the system prompt prefix stable while still delivering
    /// mid-session memory changes to the LLM on every user turn.
    async fn drain_memory_queue(&self) -> String {
        let mut queue = self.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);

        let mut xml = String::with_capacity(512);
        xml.push_str("<memory-update>\n");
        for update in &updates {
            xml.push_str(update);
            xml.push('\n');
        }
        xml.push_str("</memory-update>");
        xml
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("");
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        match method {
            "chat" => {
                let message = request["params"]["message"].as_str().unwrap_or("");
                info!(message = %message, "Chat request received");

                // --- SelfField review: gate the user message before LLM ---
                let intent = Intent {
                    action: "chat".to_string(),
                    parameters: serde_json::json!({ "message": message }),
                    source: IntentSource::User,
                    description: format!(
                        "User chat message: {}",
                        &message[..message.len().min(80)]
                    ),
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
                let memory_update = self.drain_memory_queue().await;
                let mut effective_message = String::new();

                // Memory updates first (if any)
                if !memory_update.is_empty() {
                    effective_message.push_str(&memory_update);
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

                effective_message.push_str(message);

                // --- PreTurn hooks ---
                {
                    // Gather session info before locking hook_registry
                    let (session_id, turn_count) = {
                        let sm = self.session_manager.lock().await;
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
                    let mut sm = self.session_manager.lock().await;
                    if sm.turn_count() == 0 {
                        sm.push_system(&system_prompt);
                    }
                    sm.push_user(&effective_message).await;
                }
                // Persist user message to recall memory
                {
                    let session_id = self.session_manager.lock().await.session_id.clone();
                    let rm = self.recall_memory.lock().await;
                    let _ = rm.store(&session_id, "user_message", message, None);
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
                let working_dir = std::env::current_dir().unwrap_or_default();
                let session_id = self.session_manager.lock().await.session_id.clone();

                let execute_tool = move |_id: &str, name: &str, input: &serde_json::Value| {
                    let runner = runner.clone();
                    let tools_arc = tools_arc.clone();
                    let name = name.to_string();
                    let input = input.clone();
                    let working_dir = working_dir.clone();
                    let session_id = session_id.clone();
                    async move {
                        let tool = {
                            let reg = tools_arc.lock().await;
                            reg.get(&name).cloned()
                        };
                        let exec_ctx = aletheon_abi::tool::ToolContext {
                            working_dir,
                            session_id,
                        };
                        match tool {
                            Some(t) => {
                                let mut r = runner.lock().await;
                                let res = r.run(t.as_ref(), input, &exec_ctx, "chat-turn").await;
                                (res.content, res.is_error)
                            }
                            None => (format!("Unknown tool: {}", name), true),
                        }
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
                let llm = self.llm.clone();

                let mut rt = AletheonRuntime::new(self.state.lock().await.runtime.config().clone());
                let mut react_task = tokio::spawn(async move {
                    rt.process_react(
                        &effective_message,
                        &sf_ctx,
                        |_intent: &Intent, _c: &AbiContext| Ok::<_, anyhow::Error>(Verdict::Allow),
                        &*llm,
                        &tool_defs,
                        execute_tool,
                    )
                    .await
                });

                // Pump approval requests while the ReAct loop is running.
                // When a tool needs L2+ approval, the SocketApprovalGate sends
                // a PendingApproval through the channel. We generate an
                // approval_id, store the oneshot sender, and notify the client.
                let text = loop {
                    tokio::select! {
                        result = &mut react_task => {
                            // ReAct loop finished — drain any remaining approvals
                            // (they get auto-denied by the 120s timeout in the gate).
                            break result.unwrap_or_else(|e| Err(anyhow::anyhow!("react task panicked: {e}")));
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

                let text = text.unwrap_or_else(|e| format!("error: {e}"));
                info!(len = text.len(), "ReAct loop completed");

                // Narrate the completed interaction in the SelfField narrative layer (bus-aware)
                self.sf_narrate(
                    "chat_completed",
                    &format!(
                        "User asked: '{}...' | Response: {} chars",
                        &message[..message.len().min(60)],
                        text.len(),
                    ),
                )
                .await;

                // --- PostTurn hooks ---
                {
                    // Gather session info before locking hook_registry
                    let (session_id, turn_count) = {
                        let sm = self.session_manager.lock().await;
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

                // Push assistant response and compact if needed
                let turn = {
                    let mut sm = self.session_manager.lock().await;
                    sm.push_assistant(&text).await;
                    let _ = sm.compact_if_needed(&*self.llm).await;
                    sm.turn_count()
                };
                // Persist assistant response to recall memory
                {
                    let session_id = self.session_manager.lock().await.session_id.clone();
                    let rm = self.recall_memory.lock().await;
                    let _ = rm.store(&session_id, "assistant_message", &text, None);
                }

                // Enhanced reflection: analyze question and response quality
                let task_summary = if message.len() > 100 {
                    format!("{}...", &message[..100])
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
                                if let Some(evo_entry) = ExperienceSummarizer::summarize(&recent) {
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

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "response": text, "turn": turn }
                })
            }
            "clear" => {
                let mut state = self.state.lock().await;
                state.pending_input = None;
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "status": "ok" }
                })
            }
            "reflect" => {
                let reflections = self.episodic_memory.lock().await.recall_reflections(10);
                match reflections {
                    Ok(entries) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "reflections": entries }
                        })
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to recall reflections");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32001, "message": format!("Reflection recall error: {}", e) }
                        })
                    }
                }
            }
            "status" => {
                let state = self.state.lock().await;
                let session_id = state.runtime.config().session_id.clone();
                let iteration = state.runtime.iteration();
                drop(state);
                let turn_count = self.session_manager.lock().await.turn_count();

                // Reflection and evolution counts from episodic memory
                let reflection_count = self
                    .episodic_memory
                    .lock()
                    .await
                    .reflection_count()
                    .unwrap_or(0);
                let evolution_count = self
                    .episodic_memory
                    .lock()
                    .await
                    .evolution_log_count()
                    .unwrap_or(0);

                // Care weights, boundary rules, and attention from SelfField
                let sf = self.self_field.lock().await;
                let care_weights: Vec<serde_json::Value> = sf
                    .care()
                    .all_cares()
                    .into_iter()
                    .map(|c| json!({ "topic": c.topic, "weight": c.weight }))
                    .collect();
                let boundary_total = sf.boundary().rule_count();
                let boundary_immutable = sf.boundary().immutable_rule_count();
                let attention_focus = sf
                    .attention()
                    .current_focus()
                    .map(|f| f.topic)
                    .unwrap_or_default();
                drop(sf);

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "status": {
                            "session_id": session_id,
                            "turn_count": turn_count,
                            "iteration": iteration,
                            "reflection_count": reflection_count,
                            "evolution_count": evolution_count,
                            "care_weights": care_weights,
                            "boundary_rules": boundary_total,
                            "boundary_immutable": boundary_immutable,
                            "attention_focus": attention_focus,
                        }
                    }
                })
            }
            "genome" => {
                // Read the genome dynamically from SelfField using SelfReader.
                let self_field = self.self_field.lock().await;
                let reader = SelfReader::new();
                match reader.read_genome(&*self_field).await {
                    Ok(genome) => match serde_yaml::to_string(&genome) {
                        Ok(yaml) => {
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": { "genome": yaml }
                            })
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to serialize genome to YAML");
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32004, "message": format!("Genome serialization error: {}", e) }
                            })
                        }
                    },
                    Err(e) => {
                        warn!(error = %e, "Failed to read genome from SelfField");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32004, "message": format!("Genome read error: {}", e) }
                        })
                    }
                }
            }
            "evolution" => {
                // Return recent evolution log entries from episodic memory.
                match self.episodic_memory.lock().await.recall_evolution_logs(20) {
                    Ok(entries) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "evolution": entries,
                                "current_version": "0.1.0"
                            }
                        })
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to recall evolution logs");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32002, "message": format!("Evolution recall error: {}", e) }
                        })
                    }
                }
            }
            "reflect_now" => {
                // Run an immediate reflection on the current session state
                let (turn, session_id, iteration) = {
                    let state = self.state.lock().await;
                    let session_id = state.runtime.config().session_id.clone();
                    let iteration = state.runtime.iteration();
                    drop(state);
                    let turn = self.session_manager.lock().await.turn_count();
                    (turn, session_id, iteration)
                };

                let task_summary = format!(
                    "Session {} after {} turns (iteration {})",
                    session_id, turn, iteration
                );

                let mut what_worked = Vec::new();
                let mut what_failed = Vec::new();
                let mut learned = Vec::new();

                what_worked.push(format!("Session is active with {} turns", turn));
                what_worked.push(format!("Runtime iteration count: {}", iteration));

                if turn == 0 {
                    what_failed.push("No chat turns recorded yet".to_string());
                }

                // Check if there are recent reflections to draw from
                match self.episodic_memory.lock().await.recall_reflections(5) {
                    Ok(recent) if !recent.is_empty() => {
                        learned.push(format!("Reviewed {} recent reflections", recent.len()));
                        // Aggregate failure patterns
                        let failure_count: usize = recent.iter().map(|r| r.what_failed.len()).sum();
                        if failure_count > 0 {
                            what_failed.push(format!(
                                "{} failure items across recent reflections",
                                failure_count
                            ));
                        }
                    }
                    Ok(_) => {
                        learned.push("No prior reflections available for context".to_string());
                    }
                    Err(e) => {
                        what_failed.push(format!("Could not recall reflections: {}", e));
                    }
                }

                let has_failures = !what_failed.is_empty() || turn == 0;
                let entry = self.reflector.reflect_conversation(
                    &task_summary,
                    ReflectionTrigger::Manual,
                    !has_failures,
                    what_worked,
                    what_failed,
                    learned,
                );
                if let Err(e) = self.episodic_memory.lock().await.store_reflection(&entry) {
                    warn!(error = %e, "Failed to store manual reflection");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32003, "message": format!("Reflect now error: {}", e) }
                    })
                } else {
                    info!(id = %entry.id, "Manual reflection stored via reflect_now");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "reflection": {
                                "id": entry.id,
                                "timestamp": entry.timestamp.to_rfc3339(),
                                "task_summary": entry.task_summary,
                                "outcome": entry.outcome.to_string(),
                                "what_worked": entry.what_worked,
                                "what_failed": entry.what_failed,
                                "learned": entry.learned,
                                "confidence": entry.confidence,
                                "turn_count": turn,
                            }
                        }
                    })
                }
            }
            "sessions" => match SessionStore::new(&self.data_dir) {
                Ok(store) => match store.list_sessions() {
                    Ok(ids) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "sessions": ids }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32020, "message": format!("Session list error: {}", e) }
                    }),
                },
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": format!("SessionStore init error: {}", e) }
                }),
            },
            "resume" => {
                let target_session_id = request["params"]["session_id"].as_str().unwrap_or("");
                if target_session_id.is_empty() {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32021, "message": "Missing session_id parameter" }
                    })
                } else {
                    match SessionManager::recover(&self.data_dir, target_session_id).await {
                        Some(msgs) => {
                            match SessionManager::new(
                                &self.data_dir,
                                target_session_id.to_string(),
                                100_000,
                            )
                            .await
                            {
                                Ok(new_sm) => {
                                    let msg_count = msgs.len();
                                    *self.session_manager.lock().await = new_sm;
                                    info!(
                                        session_id = target_session_id,
                                        messages = msg_count,
                                        "Session resumed"
                                    );
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "session_id": target_session_id,
                                            "recovered_messages": msg_count,
                                        }
                                    })
                                }
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32021, "message": format!("SessionManager init error: {}", e) }
                                }),
                            }
                        }
                        None => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32021, "message": format!("No recoverable session: {}", target_session_id) }
                        }),
                    }
                }
            }
            "compact" => {
                let did_compact = {
                    let mut sm = self.session_manager.lock().await;
                    // Force compaction by temporarily lowering threshold
                    sm.force_compact(&*self.llm).await
                };
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "compacted": did_compact }
                })
            }
            "reload_skills" => {
                let count = {
                    let mut loader = self.skill_loader.lock().await;
                    loader.reload()
                };
                info!(count = count, "Skills reloaded via reload_skills RPC");

                // Rebuild the cached prefix with updated skills.
                // Note: core_memory snapshot is from boot; mid-session memory
                // changes ride the memory_queue, not the prefix.
                {
                    let loader = self.skill_loader.lock().await;
                    let cm = self.core_memory.lock().await;
                    let old_prefix = self.cached_prefix.lock().await;
                    let new_prefix =
                        PrefixBuilder::build(&self.config_prompt, loader.skills(), &cm);
                    if let Some(reason) = PrefixBuilder::diff_reason(&old_prefix, &new_prefix) {
                        info!(reason = %reason, "Prefix changed after skill reload (cache will miss)");
                    }
                    drop(old_prefix);
                    drop(cm);
                    drop(loader);
                    *self.cached_prefix.lock().await = new_prefix;
                }

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "skills_loaded": count }
                })
            }
            "approval_response" => {
                // Resolve a pending approval request. The client sends this
                // in response to an "approval_request" notification.
                let aid = request["params"]["approval_id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let decision = match request["params"]["decision"].as_str().unwrap_or("deny") {
                    "approve" => ApprovalDecision::Approve,
                    "approve_for_session" => ApprovalDecision::ApproveForSession,
                    _ => ApprovalDecision::Deny,
                };
                if let Some(tx) = self.pending_approvals.lock().await.remove(&aid) {
                    let _ = tx.send(decision);
                    info!(approval_id = %aid, decision = ?request["params"]["decision"], "Approval resolved");
                } else {
                    warn!(approval_id = %aid, "No pending approval found for id");
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": true }
                })
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}
