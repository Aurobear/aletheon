mod chat;
mod format;
mod rpc;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio_util::sync::CancellationToken;

use super::model_router::{ModelRouter, TaskType};
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
use base::envelope::*;
use base::hook::{HookContext, HookPoint, HookResult};
use base::Registry;
use base::{
    Context as AbiContext, Intent, IntentSource, ReflectionTrigger, SelfFieldOps, Subsystem,
    SubsystemContext, Verdict,
};
use base::ui_event::{CollaborationMode, InterruptReason};
use corpus::security::sandbox::executor::{SandboxExecutor, SandboxPreference};
use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::audit::AuditLogger;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::{PendingApproval, SocketApprovalGate};
use corpus::tools::tools::Tool;
use corpus::tools::tools::ToolRegistry;
use cognit::core::reflector::Reflector;
use cognit::core::ExperienceSummarizer;
use cognit::r#impl::llm::LlmProvider;
use base::envelope::Payload;
use base::CommunicationBus;
use memory::episodic::EpisodicMemory;
use metacog::r#impl::meta_runtime::self_reader::SelfReader;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use crate::core::evolution_coordinator::EvolutionConfig;
use base::Version;
use dasein::r#impl::perception::bridge::PerceptionInjection;
use dasein::{SelfField, SelfFieldConfig};
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};

use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};
use crate::core::checkpoint::CheckpointStore;
use crate::core::event_sink::{ChannelEventSink, Event};
use crate::core::react_loop::{ReActLoop, TurnMetrics};
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::hooks::builtin::audit_hook;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::agent_loader::AgentLoader;
use crate::core::config::HooksConfig;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;
use crate::r#impl::skills::plugin::register_skill;

use super::debug_handler::DebugHandler;
use super::prefix_builder::PrefixBuilder;
use super::DaemonConfig;
use base::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};
use crate::core::session_gateway::{ParamRegistry, SessionGateway};
use crate::core::session_gateway::gateway::SessionStateRef;

pub(crate) use format::event_to_json;

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
    model_router: Arc<ModelRouter>,
    session_manager: Arc<Mutex<SessionManager>>,
    recall_memory: Arc<Mutex<RecallMemory>>,
    data_dir: PathBuf,
    /// The LLM's context window size, used for SessionManager creation.
    context_window: usize,
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
    /// SQLite-backed fact store with trust scoring and FTS5 search.
    fact_store: Arc<Mutex<FactStore>>,
    /// SQLite-backed objective store for persistent goal tracking.
    objective_store: Arc<Mutex<ObjectiveStore>>,
    /// Cached active objective + sub-goals for resume-on-start.
    /// Applied once to GoalTracker before the first chat turn.
    resumed_objective: Option<(String, Vec<String>)>,
    /// Loop detector: tracks consecutive tool failures/successes.
    storm_breaker: Arc<Mutex<StormBreaker>>,
    /// Per-session checkpoint store for file-edit rewind.
    #[allow(dead_code)]
    checkpoint_store: Arc<Mutex<CheckpointStore>>,
    /// Keyword-based skill router for prompt-to-skill matching.
    skill_router: Arc<Mutex<SkillRouter>>,
    /// Agent role loader — loads agent markdown definitions from ~/.aletheon/agents/.
    #[allow(dead_code)]
    agent_loader: Arc<Mutex<AgentLoader>>,
    /// Configured hook scripts from the [hooks] config section.
    hooks_config: HooksConfig,
    /// Per-session "always approve" cache: tool_name -> approved.
    /// Populated when user responds with "always" to an approval request.
    session_approvals: Arc<Mutex<HashMap<String, bool>>>,
    /// Morphogenesis pipeline for post-turn evolution.
    pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
    /// Automatic memory extraction — uses a cheap LLM to extract and store
    /// important facts from each conversation turn.
    auto_memory: Arc<Mutex<AutoMemory>>,
    /// Debug handler — exposes debug.* JSON-RPC methods for tracing, perf, and bag recording.
    debug_handler: Arc<DebugHandler>,
    /// Performance counter — shared with DebugHandler, also used by the ReAct loop.
    debug_perf: Arc<PerfCounter>,
    /// Cancellation token for the current chat turn.
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    /// EventBus for cross-subsystem event routing (DaseinEventBridge, etc.).
    event_bus: Option<Arc<dyn base::EventBus>>,
    /// Daemon-level cancellation token for graceful shutdown via daemon.shutdown RPC.
    daemon_cancel_token: Option<CancellationToken>,
    /// Session Gateway — unified facade for external agent debug access.
    session_gateway: Arc<SessionGateway>,
}

impl RequestHandler {
    /// Get a reference to the debug handler (for subscriber rx access).
    pub fn debug_handler(&self) -> &Arc<DebugHandler> {
        &self.debug_handler
    }

    /// Get a reference to the tool registry (for MCP server).
    pub fn tools(&self) -> Arc<Mutex<ToolRegistry>> {
        self.tools.clone()
    }

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
        model_routing: crate::core::config::ModelRoutingConfig,
        evolution_enabled: bool,
        _perception_rx: mpsc::Receiver<PerceptionInjection>,
        event_bus: Option<Arc<dyn base::EventBus>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create("")?);
        info!(provider = llm.name(), "LLM provider initialized");

        // Create session and journal
        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let session_store = SessionStore::new(&data_dir)?;
        session_store.create_session(&session_id)?;

        info!(session_id = %session_id, "Created new session");

        // Create SelfField for genome reads and policy engine
        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            ..Default::default()
        };
        let self_field = Arc::new(Mutex::new(SelfField::new(self_field_config)));

        // Tier 2a: install the Runtime PermissionManager as the permission authority.
        // This delegates the confirmation verdict from dasein's inline rule to the
        // Runtime's policy manager (behavior-identical port).
        {
            use crate::core::permission_manager::PermissionManager;
            let mut sf = self_field.lock().await;
            sf.set_permission_authority(std::sync::Arc::new(PermissionManager::new()));
        }

        // Wire DaseinEventBridge to EventBus if available
        if let Some(ref eb) = event_bus {
            let sf = self_field.lock().await;
            sf.wire_dasein_event_bridge(&**eb).await?;
        }

        // Create memory instances
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db_path = data_dir.join("recall_memory.db");
        let recall_memory = Arc::new(Mutex::new(RecallMemory::new(&recall_db_path)?));

        // FactStore — structured long-term memory with trust scoring
        let aletheon_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aletheon");
        std::fs::create_dir_all(&aletheon_dir)?;
        let fact_store = FactStore::open(&aletheon_dir.join("fact_store.db"))
            .context("opening fact store")?;
        let fact_store = Arc::new(Mutex::new(fact_store));

        // ObjectiveStore — persisted goals with resume-on-start support
        let objective_store = ObjectiveStore::open(&aletheon_dir.join("objectives.db"))
            .context("opening objective store")?;
        let objective_store = Arc::new(Mutex::new(objective_store));

        // Resume active objective for session continuity
        let resumed_objective = {
            let store = objective_store.lock().await;
            match store.resume() {
                Ok(Some((obj, subs))) => {
                    let sub_desc: Vec<String> =
                        subs.iter().map(|s| s.description.clone()).collect();
                    info!(
                        objective_id = obj.objective_id,
                        description = %obj.description,
                        sub_goals = sub_desc.len(),
                        "Resuming persisted objective on start"
                    );
                    Some((obj.description.clone(), sub_desc))
                }
                Ok(None) => {
                    info!("No active objective to resume — fresh start");
                    None
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read active objective on start");
                    None
                }
            }
        };

        // Use the LLM provider's actual context window size for session management.
        // This replaces the hardcoded 100_000 with the real model limit.
        let context_window = llm.max_context_length();
        let session_manager = SessionManager::new(
            &data_dir,
            session_id.clone(),
            context_window,
        )
        .await?;
        info!(context_window = context_window, "Session context window configured");
        let session_manager = Arc::new(Mutex::new(session_manager));

        // Register tools including memory tools
        let mut tools = ToolRegistry::default();
        let _ = tools.register(Arc::new(CoreMemoryAppendTool {
            memory: core_memory.clone(),
        }));
        let _ = tools.register(Arc::new(CoreMemoryReplaceTool {
            memory: core_memory.clone(),
        }));
        let _ = tools.register(Arc::new(MemorySearchTool {
            recall: recall_memory.clone(),
            core_memory: core_memory.clone(),
            fact_store: Some(fact_store.clone()),
        }));

        // Connect configured MCP servers and register their tools alongside built-ins.
        {
            let mcp_config = corpus::tools::mcp::config::McpConfig {
                servers: config.mcp_servers.clone(),
                ..Default::default()
            };
            let mut mcp = corpus::tools::mcp::manager::McpManager::new(mcp_config);
            if let Err(e) = mcp.connect_all().await {
                tracing::warn!(error = %e, "MCP connect_all failed; continuing without MCP tools");
            }
            let mcp_count = mcp.connected_count();
            if mcp_count > 0 {
                info!(servers = mcp_count, "MCP servers connected");
            }
            for wrapper in mcp.tool_wrappers() {
                let name = wrapper.name().to_string();
                use base::Registry;
                if let Err(e) = tools.register(Arc::from(wrapper)) {
                    tracing::warn!(tool = %name, error = %e, "skip MCP tool (name clash?)");
                } else {
                    info!(tool = %name, "Registered MCP tool");
                }
            }
        }

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
            context_window_tokens: context_window,
            ..Default::default()
        };
        let runtime_config_snapshot = runtime_config.clone();

        // Create agent registry: try config-based loading first, fall back to builtins
        let agents_dir = PathBuf::from("agents");

        // Collect default tools for config-based agent loading
        let default_tools: Vec<Box<dyn Tool>> = vec![
            Box::new(corpus::tools::tools::file_read::FileReadTool),
            Box::new(corpus::tools::tools::file_write::FileWriteTool),
            Box::new(corpus::tools::tools::bash_exec::BashExecTool),
            Box::new(corpus::tools::tools::system_status::SystemStatusTool),
            Box::new(corpus::tools::tools::process_list::ProcessListTool),
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

        let mut runtime = AletheonRuntime::new(runtime_config);

        // Wire EvolutionCoordinator for post-turn self-evolution.
        // HIGH-risk autonomy: OFF unless config.evolution.enabled is true.
        // TODO(Tier 2a): additionally gate migrations behind PermissionManager.
        let evo_config = EvolutionConfig {
            enabled: evolution_enabled,
            trigger_every_n_turns: 10,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: data_dir.join("lineage"),
        };
        runtime = runtime.with_evolution(evo_config)?;

        // Resume persisted objective into the goal tracker (resume-on-start)
        if let Some((ref desc, ref subs)) = resumed_objective {
            runtime.seed_goal(desc, subs);
        }

        // Create morphogenesis pipeline for evolution coordinator
        let meta_runtime = DefaultMetaRuntime::new(Version::new(0, 1, 0));
        let pipeline = Arc::new(MorphogenesisPipeline::new(meta_runtime));

        // Create reflector and episodic memory for post-chat reflection
        let reflector = Reflector::new();
        let episodic_db_path = data_dir.join("episodic.db");
        let mut episodic_memory = EpisodicMemory::new(episodic_db_path);
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: data_dir.clone(),
            config: serde_json::Value::Null,
            bus: Arc::new(base::CommunicationBus::new()),
        };
        episodic_memory.init(&ctx).await?;
        let episodic_memory = Arc::new(Mutex::new(episodic_memory));

        // Initialize skill loader from the default skills directory
        let skills_dir = base::paths::skills_dir();
        let mut skill_loader = SkillLoader::new(skills_dir);
        let loaded = skill_loader.load_all_enhanced();
        if loaded > 0 {
            info!(count = loaded, "Skills loaded at startup");
        }

        // Create hook registry and register builtin hooks
        let mut hook_registry = HookRegistry::new();
        audit_hook::register_audit_hook(&mut hook_registry);

        // Load user hooks from ~/.aletheon/hooks/
        let hooks_dir = aletheon_dir.join("hooks");
        let hook_loader = crate::r#impl::hooks::loader::HookLoader::new(hooks_dir);
        let user_hook_count = hook_loader.register_all(&mut hook_registry);
        if user_hook_count > 0 {
            info!(count = user_hook_count, "Loaded user hooks");
        }

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

        // ── AgentTool — sub-agent delegation ───────────────────────────────────────
        {
            let agents_dir = aletheon_dir.join("agents");
            let mut rt_agent_loader = crate::r#impl::agent_loader::AgentLoader::new();
            if agents_dir.exists() {
                let _ = rt_agent_loader.load_from_dir(&agents_dir);
            }

            let mut agent_defs: std::collections::HashMap<String, corpus::tools::tools::agent_tool::AgentDefinition> = std::collections::HashMap::new();
            for role in rt_agent_loader.list() {
                agent_defs.insert(
                    role.name.clone(),
                    corpus::tools::tools::agent_tool::AgentDefinition {
                        name: role.name.clone(),
                        description: role.description.clone(),
                        tools: role.tools.clone(),
                        model: role.model.clone(),
                        max_iterations: 20,
                        system_prompt: role.body.clone(),
                    },
                );
            }

            if !agent_defs.is_empty() {
                let llm_for_agents: Arc<dyn cognit::r#impl::llm::LlmProvider> = llm.clone();
                let tools_for_agents = tools.clone();
                let execute_fn: corpus::tools::tools::agent_tool::ExecuteSubAgentFn = Arc::new(
                    move |system_prompt: String, user_prompt: String, allowed_tools: Vec<String>| {
                        let llm = llm_for_agents.clone();
                        let tools = tools_for_agents.clone();
                        Box::pin(async move {
                            // Filter tool registry to only allowed tools
                            let reg = tools.lock().await;
                            let agent_tool_defs: Vec<base::ToolDefinition> = reg
                                .definitions()
                                .into_iter()
                                .filter(|d| allowed_tools.contains(&d.name))
                                .collect();
                            drop(reg);

                            // Build messages for the LLM
                            let mut current_messages = vec![
                                base::message::Message::system(&system_prompt),
                                base::message::Message::user(&user_prompt),
                            ];

                            // ReAct loop: up to 20 iterations
                            let mut response_text = String::new();
                            for _ in 0..20 {
                                let response = llm.complete(&current_messages, &agent_tool_defs).await?;

                                // Extract text and tool calls from response
                                let mut text_parts = Vec::new();
                                let mut tool_calls = Vec::new();
                                for block in &response.content {
                                    match block {
                                        base::message::ContentBlock::Text { text } => {
                                            text_parts.push(text.clone());
                                        }
                                        base::message::ContentBlock::ToolUse { id, name, input } => {
                                            tool_calls.push((id.clone(), name.clone(), input.clone()));
                                        }
                                        _ => {}
                                    }
                                }

                                if tool_calls.is_empty() {
                                    response_text = text_parts.join("\n");
                                    break;
                                }

                                // Add assistant message verbatim (text + tool_use blocks)
                                current_messages.push(base::message::Message {
                                    role: base::message::Role::Assistant,
                                    content: response.content.clone(),
                                });

                                // Execute each tool call
                                for (id, name, input) in tool_calls {
                                    let reg = tools.lock().await;
                                    let result = if let Some(tool) = reg.get(&name) {
                                        let ctx = base::tool::ToolContext {
                                            working_dir: std::env::current_dir().unwrap_or_default(),
                                            session_id: "sub-agent".into(),
                                        };
                                        tool.execute(input, &ctx).await
                                    } else {
                                        base::tool::ToolResult {
                                            content: format!("Unknown tool: {}", name),
                                            is_error: true,
                                            metadata: base::tool::ToolResultMeta::default(),
                                        }
                                    };
                                    drop(reg);

                                    // Add tool result as user message
                                    current_messages.push(base::message::Message::tool_result(
                                        &id,
                                        &result.content,
                                        result.is_error,
                                    ));
                                }
                            }

                            Ok(response_text)
                        })
                    },
                );

                let agent_tool = corpus::tools::tools::agent_tool::AgentTool::new(
                    agent_defs.clone(),
                    execute_fn,
                );
                if let Err(e) = tools.lock().await.register(Arc::new(agent_tool)) {
                    tracing::warn!(error = %e, "Failed to register AgentTool");
                } else {
                    info!(agents = agent_defs.len(), "Registered AgentTool with sub-agents");
                }
            }
        }

        // ── StormBreaker ──────────────────────────────────────────────────────
        let storm_breaker = Arc::new(Mutex::new(StormBreaker::new(3)));

        // ── CheckpointStore ────────────────────────────────────────────────────
        let session_dir = aletheon_dir.join("sessions").join(&session_id);
        std::fs::create_dir_all(&session_dir)?;
        let checkpoint_store = CheckpointStore::new(&session_dir);
        let checkpoint_store = Arc::new(Mutex::new(checkpoint_store));

        // ── SkillRouter ───────────────────────────────────────────────────────
        let mut skill_router = SkillRouter::new();
        let skills_dirs = vec![
            aletheon_dir.join("skills"),
            PathBuf::from(".aletheon/skills"),
        ];
        for dir in &skills_dirs {
            if dir.exists() {
                let _ = skill_router.load_from_dir(dir);
            }
        }
        let skill_router = Arc::new(Mutex::new(skill_router));

        // ── AgentLoader ───────────────────────────────────────────────────────
        let mut agent_loader = AgentLoader::new();
        let agents_dir = aletheon_dir.join("agents");
        if agents_dir.exists() {
            let _ = agent_loader.load_from_dir(&agents_dir);
            info!("Loaded {} agent roles", agent_loader.list().len());
        }
        let agent_loader = Arc::new(Mutex::new(agent_loader));

        // ── HooksConfig ───────────────────────────────────────────────────────
        let hooks_config = config.hooks.clone();

        // Create ModelRouter for dynamic model selection
        let model_router = Arc::new(ModelRouter::new(
            model_routing.clone(),
            Arc::new(registry.clone()),
        ));
        info!(
            default = %model_router.model_name_for(TaskType::General),
            multimodal = %model_router.model_name_for(TaskType::Multimodal),
            cheap = %model_router.model_name_for(TaskType::Simple),
            reasoning = %model_router.model_name_for(TaskType::Reasoning),
            "ModelRouter initialized"
        );

        // ── AutoMemory ──────────────────────────────────────────────────────
        // Use ModelRouter to get the AutoMemory model (cheap model for fact extraction)
        let cheap_llm: Arc<dyn LlmProvider> = match model_router.create_provider(TaskType::AutoMemory) {
            Ok(provider) => {
                info!(model = provider.name(), "AutoMemory using routed model");
                Arc::from(provider)
            }
            Err(e) => {
                tracing::warn!(error = %e, "ModelRouter AutoMemory failed, falling back to default");
                Arc::from(registry.resolve_and_create("").expect("no LLM available"))
            }
        };
        let auto_memory = Arc::new(Mutex::new(AutoMemory::new(
            cheap_llm,
            core_memory.clone(),
        )));
        info!("AutoMemory initialized with routed extraction model");

        // ── Debug infrastructure ──────────────────────────────────────────
        let debug_perf = Arc::new(PerfCounter::default());
        let debug_hook = Arc::new(tokio::sync::Mutex::new(DebugBusHook::new(EventFilter::default())));
        let debug_handler = Arc::new(DebugHandler::new(debug_hook, debug_perf.clone()));
        info!("DebugHandler initialized");

        // ── Session Gateway ─────────────────────────────────────────────
        let param_registry = Arc::new(ParamRegistry::new());
        let gw_state = Arc::new(Mutex::new(SessionStateRef {
            iteration: 0,
            plan_mode: false,
            consecutive_errors: 0,
            circuit_breaker_status: crate::core::react_loop::circuit_breaker::CircuitBreakerStatus::Ok,
            tool_budget_remaining: runtime_config_snapshot.agent_loop.max_tool_calls,
            tool_budget_max: runtime_config_snapshot.agent_loop.max_tool_calls,
            recent_tools: Vec::new(),
            storm_breaker_failure_count: 0,
            goal_tracker: crate::core::react_loop::goal_tracker::GoalTracker::new(),
        }));
        let gw_started_at = std::time::Instant::now();
        let session_gateway = Arc::new(SessionGateway::new(
            param_registry.clone(),
            debug_handler.clone(),
            session_id.clone(),
            gw_state.clone(),
            session_manager.clone(),
            gw_started_at,
            runtime_config_snapshot,
            core_memory.clone(),
            recall_memory.clone(),
            self_field.clone(),
            llm.clone(),
        ));
        info!("SessionGateway initialized");

        let handler = Self {
            state: Arc::new(Mutex::new(SessionState {
                runtime,
                pending_input: None,
            })),
            llm,
            model_router,
            session_manager: session_manager.clone(),
            recall_memory,
            data_dir,
            context_window,
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
            fact_store,
            objective_store,
            resumed_objective,
            storm_breaker,
            checkpoint_store,
            skill_router,
            agent_loader,
            hooks_config,
            session_approvals: Arc::new(Mutex::new(HashMap::new())),
            pipeline,
            auto_memory,
            debug_handler,
            debug_perf,
            cancel_token: Arc::new(Mutex::new(None)),
            event_bus,
            daemon_cancel_token: Some(cancel_token),
            session_gateway,
        };

        // ── Register initial params ──────────────────────────────────────
        {
            let data_dir_clone = handler.data_dir.clone();
            let started_at = std::time::Instant::now();
            param_registry.declare(
                "session.uptime_secs",
                "session",
                "Daemon uptime in seconds",
                move || json!(started_at.elapsed().as_secs()),
            ).await;
            param_registry.declare(
                "session.data_dir",
                "session",
                "Data directory path",
                move || json!(data_dir_clone.to_string_lossy()),
            ).await;
            let model = config.model.clone();
            param_registry.declare(
                "llm.model",
                "llm",
                "Current LLM model in use",
                move || json!(model),
            ).await;
            let provider_name = handler.llm.name().to_string();
            param_registry.declare(
                "llm.provider",
                "llm",
                "Current LLM provider name",
                move || json!(provider_name),
            ).await;
            let sandbox_pref = config.sandbox_preference.clone();
            param_registry.declare(
                "sandbox.preference",
                "sandbox",
                "Current sandbox mode",
                move || json!(sandbox_pref),
            ).await;
            param_registry.declare(
                "session.rss_kb",
                "session",
                "Resident memory in KB",
                || {
                    let status = std::fs::read_to_string("/proc/self/status").ok();
                    let rss = status.and_then(|s| {
                        s.lines()
                            .find(|l| l.starts_with("VmRSS:"))
                            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                    });
                    json!(rss.unwrap_or(0))
                },
            ).await;
            info!("Registered {} initial params", 6);
        }

        // Fire OnSessionStart hook
        {
            let hr = handler.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnSessionStart,
                session_id: session_id.clone(),
                turn_count: 0,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        Ok(handler)
    }

    /// Review an intent through SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    pub(crate) async fn sf_review(&self, intent: &Intent, ctx: &AbiContext) -> anyhow::Result<Verdict> {
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
    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
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

    /// Post-turn coordination: update Dasein mood from turn output.
    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.self_field.lock().await;
        if let Some(ref dasein) = sf.dasein() {
            let _mood = dasein.quick_mood_update(turn_text);
            tracing::info!(turn = turn, "Dasein mood updated via coordinator");
        }
    }

    /// Compose the user message with mid-session injections from the memory queue.
    ///
    /// Drains all pending memory updates and prepends them as a `<memory-update>`
    /// XML block before the raw user input.  This is the same pattern as
    /// `ReActLoop::compose_user_message()` and `Controller::compose_user_message()`
    /// — changes ride the user message tail so the system prompt prefix stays
    /// byte-stable for provider cache hits.
    ///
    /// Returns empty string if the queue is empty (no injections needed).
    pub(crate) async fn compose_memory_block(&self) -> String {
        let mut queue = self.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);

        let items: Vec<String> = updates.iter().map(|m| format!("- {}", m)).collect();
        format!("<memory-update>\n{}\n</memory-update>", items.join("\n"))
    }

    /// Execute configured hook scripts at a lifecycle point.
    ///
    /// Each script is spawned as a subprocess with `input_json` piped to stdin.
    /// Stdout from successful scripts is collected and returned.
    /// Each script has a 30-second timeout.
    pub(crate) async fn run_hook_scripts(&self, scripts: &[String], input_json: &str) -> Vec<String> {
        let mut outputs = Vec::new();
        for script_path in scripts {
            let path = format::expand_tilde(script_path);
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
                    // Write input to stdin
                    if let Some(stdin) = child.stdin.take() {
                        let input = input_json.to_string();
                        tokio::spawn(async move {
                            use tokio::io::AsyncWriteExt;
                            let mut stdin = stdin;
                            let _ = stdin.write_all(input.as_bytes()).await;
                        });
                    }
                    // Capture stdout before waiting
                    let mut stdout_pipe = child.stdout.take();
                    // Wait with 30-second timeout
                    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
                        Ok(Ok(status)) if status.success() => {
                            // Read captured stdout
                            if let Some(ref mut stdout) = stdout_pipe {
                                use tokio::io::AsyncReadExt;
                                let mut buf = String::new();
                                if stdout.read_to_string(&mut buf).await.is_ok() && !buf.is_empty() {
                                    outputs.push(buf);
                                }
                            }
                        }
                        Ok(Ok(status)) => {
                            tracing::warn!(
                                path = %path,
                                code = status.code(),
                                "Hook script exited with non-zero status"
                            );
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
        outputs
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("").to_string();
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let params = request.get("params").cloned().unwrap_or(serde_json::Value::Null);

        // Route session.* methods to the Session Gateway (new unified facade).
        if method.starts_with("session.") {
            if let Some(response) = self.session_gateway.handle_method(&method, &id, &params).await {
                return response;
            }
        }

        // Route debug.* methods to the debug handler (backward compat).
        if method.starts_with("debug.") {
            if let Some(response) = self.debug_handler.handle_method(&method, &id, &params).await {
                return response;
            }
        }

        match method.as_str() {
            "chat" => self.handle_chat(id, request).await,
            _ => self.handle_rpc(&method, id, request).await,
        }
    }
}
