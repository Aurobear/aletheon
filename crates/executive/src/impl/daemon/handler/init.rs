//! Handler initialization and construction.
//!
//! Contains the `RequestHandler::new()` constructor and setup-related methods
//! (`set_notify_channel`, `create_notify_channel`, `tools`, `debug_handler`).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use fabric::Clock;
use tokio_util::sync::CancellationToken;

use super::super::model_router::{ModelRouter, TaskType};
use super::super::prefix_builder::PrefixBuilder;
use super::super::session_manager::SessionManager;
use super::super::DaemonConfig;
use super::RequestHandler;
use crate::core::config::ExecutiveConfig;
use crate::core::evolution_coordinator::EvolutionConfig;
use crate::core::orchestrator::AletheonExecutive;
use crate::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};
use crate::session::store::SessionStore;
use crate::CoreMemory;
use crate::ProviderRegistry;
use crate::RecallMemory;
use cognit::core::reflector::Reflector;
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::{create_default_executor, SandboxPreference};
use corpus::security::socket_approval::SocketApprovalGate;
use corpus::tools::tools::ToolRegistry;
use dasein::{SelfField, SelfFieldConfig};
use fabric::hook::{HookContext, HookPoint};
use fabric::CommunicationBus;
use fabric::LlmProvider;
use fabric::Registry;
use fabric::SubAgentState;
use fabric::Version;
use fabric::{Subsystem, SubsystemContext};
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use mnemosyne::episodic::EpisodicMemory;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::r#impl::agent_loader::AgentLoader;
use crate::r#impl::goal::ObjectiveStore;
use aletheon_kernel::supervision::RestartPolicy;
use corpus::hook::builtin::audit_hook;
use corpus::security::storm_breaker::StormBreaker;
use corpus::skill::plugin::register_skill;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;
use mnemosyne::AutoMemory;
use mnemosyne::FactStore;

use super::super::debug_handler::DebugHandler;
use crate::core::session_gateway::gateway::SessionStateRef;
use crate::core::session_gateway::{ParamRegistry, SessionGateway};
use crate::core::sub_agent::SubAgentRuntime;
use async_trait::async_trait;
use fabric::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

// ---------------------------------------------------------------------------
// DaemonSubAgentRuntime — production sub-agent execution
// ---------------------------------------------------------------------------

/// Production sub-agent execution runtime: single-turn LLM completion with
/// optional tool calls. Wired into [`SubAgentSpawner`] so spawned sub-agents
/// perform real reasoning work instead of the cancellation-wait stub.
struct DaemonSubAgentRuntime {
    llm: Arc<dyn LlmProvider>,
    tools: Arc<Mutex<ToolRegistry>>,
}

#[async_trait]
impl SubAgentRuntime for DaemonSubAgentRuntime {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String> {
        let messages = vec![fabric::Message::user(task)];
        // Run up to 8 reasoning steps; respect cancellation between each.
        let mut current = messages;
        let mut final_text = String::new();
        for _step in 0..8 {
            if cancel.is_cancelled() {
                return Err("sub-agent cancelled".into());
            }
            let tool_defs: Vec<fabric::ToolDefinition> = {
                let reg = self.tools.lock().await;
                reg.definitions()
            };
            let response = self
                .llm
                .complete(&current, &tool_defs)
                .await
                .map_err(|e| format!("LLM error: {e}"))?;

            let mut text_parts: Vec<String> = Vec::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            for block in &response.content {
                match block {
                    fabric::ContentBlock::Text { text } => text_parts.push(text.clone()),
                    fabric::ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            if tool_calls.is_empty() {
                final_text = text_parts.join("\n");
                break;
            }

            // Push assistant response with tool calls.
            current.push(fabric::Message {
                role: fabric::Role::Assistant,
                content: response.content.clone(),
            });

            // Execute tools and push results.
            for (call_id, name, input) in tool_calls {
                if cancel.is_cancelled() {
                    return Err("sub-agent cancelled".into());
                }
                let result = {
                    let reg = self.tools.lock().await;
                    match reg.get(&name) {
                        Some(tool) => {
                            let ctx = fabric::tool::ToolContext {
                                working_dir: std::env::current_dir().unwrap_or_default(),
                                session_id: "sub-agent".into(),
                                clock: std::sync::Arc::new(SystemClock::new()),
                            };
                            tool.execute(input, &ctx).await
                        }
                        None => fabric::tool::ToolResult {
                            content: format!("Unknown tool: {name}"),
                            is_error: true,
                            metadata: fabric::tool::ToolResultMeta::default(),
                        },
                    }
                };
                current.push(fabric::Message::tool_result(
                    &call_id,
                    &result.content,
                    result.is_error,
                ));
            }
        }
        if final_text.is_empty() {
            Ok("(sub-agent produced no text output)".into())
        } else {
            Ok(final_text)
        }
    }
}

impl RequestHandler {
    /// Get a reference to the debug handler (for subscriber rx access).
    pub fn debug_handler(&self) -> &Arc<DebugHandler> {
        &self.subsystems.debug_handler
    }

    /// Get a reference to the tool registry (for MCP server).
    pub fn tools(&self) -> Arc<Mutex<ToolRegistry>> {
        self.subsystems.corpus.tools.clone()
    }

    /// Set the notification channel for out-of-band messages to the client.
    pub fn set_notify_channel(&mut self, tx: mpsc::Sender<String>) {
        self.notify_tx = Some(tx.clone());
        // Propagate to the shared orchestrator handle
        if let Ok(mut guard) = self.turn_orchestrator.notify_tx().try_lock() {
            *guard = Some(tx);
        }
    }

    /// Create a notification channel and wire it to the handler.
    /// Returns the receiver for the server to consume out-of-band notifications.
    pub fn create_notify_channel(&mut self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(64);
        self.set_notify_channel(tx);
        rx
    }

    pub async fn new(
        config: &DaemonConfig,
        registry: &ProviderRegistry,
        model_routing: crate::core::config::ModelRoutingConfig,
        evolution_enabled: bool,
        event_bus: Option<Arc<CommunicationBus>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create("")?);
        info!(provider = llm.name(), "LLM provider initialized");

        // Create session and journal
        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data dir: {}", data_dir.display()))?;
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
        {
            use crate::core::permission_manager::PermissionManager;
            let mut sf = self_field.lock().await;
            sf.set_permission_authority(std::sync::Arc::new(PermissionManager::new()));
        }

        // Wire DaseinEventBridge to CommunicationBus if available
        if let Some(ref bus) = event_bus {
            let sf = self_field.lock().await;
            sf.wire_dasein_event_bridge(bus).await?;
        }

        // Create memory instances
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db_path = data_dir.join("recall_memory.db");
        let recall_clock: Arc<dyn fabric::Clock> = Arc::new(SystemClock::new());
        let recall_memory = Arc::new(Mutex::new(RecallMemory::new(
            &recall_db_path,
            recall_clock,
        )?));

        // FactStore
        let aletheon_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aletheon");
        std::fs::create_dir_all(&aletheon_dir)?;
        let fact_store =
            FactStore::open(&aletheon_dir.join("fact_store.db")).context("opening fact store")?;
        let fact_store = Arc::new(Mutex::new(fact_store));

        // ObjectiveStore
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
                    info!("No active objective to resume");
                    None
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read active objective on start");
                    None
                }
            }
        };

        // Multi-session setup
        let context_window = llm.max_context_length();
        let initial_session =
            SessionManager::new(&data_dir, session_id.clone(), context_window).await?;
        info!(
            context_window = context_window,
            "Session context window configured"
        );
        let initial_session = Arc::new(Mutex::new(initial_session));
        let mut sessions = HashMap::new();
        sessions.insert(session_id.clone(), initial_session.clone());
        let sessions = Arc::new(Mutex::new(sessions));
        let default_session_id = Arc::new(tokio::sync::Mutex::new(session_id.clone()));

        // Clock for monotonic/wall timestamps. Created early so
        // session_created_at timestamps are routed through Clock.
        let clock = Arc::new(SystemClock::new());

        let session_created_at = {
            let mut m = HashMap::new();
            m.insert(session_id.clone(), clock.mono_now());
            Arc::new(Mutex::new(m))
        };
        let active_connections = Arc::new(AtomicUsize::new(0));

        // Register tools
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

        // MCP servers
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
                if let Err(e) = tools.register(Arc::from(wrapper)) {
                    tracing::warn!(tool = %name, error = %e, "skip MCP tool (name clash?)");
                } else {
                    info!(tool = %name, "Registered MCP tool");
                }
            }
        }

        // Security
        let sandbox_pref = SandboxPreference::from_str(&config.sandbox_preference);
        let sandbox = create_default_executor(sandbox_pref, clock.clone());
        let audit_path = data_dir.join("audit.jsonl");
        let audit_logger = AuditLogger::new(audit_path)?;
        let (approval_gate, approval_rx) = SocketApprovalGate::new();
        let tool_runner = Arc::new(Mutex::new(
            ToolRunnerWithGuard::new(sandbox, audit_logger, clock.clone())
                .with_approval_gate(Arc::new(approval_gate)),
        ));

        let runtime_config = ExecutiveConfig {
            session_id: session_id.clone(),
            context_window_tokens: context_window,
            ..Default::default()
        };
        let runtime_config_snapshot = runtime_config.clone();

        let mut runtime = AletheonExecutive::new(runtime_config);
        let evo_config = EvolutionConfig {
            enabled: evolution_enabled,
            evolution_permitted: false,
            trigger_every_n_turns: 10,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: data_dir.join("lineage"),
        };
        runtime = runtime.with_evolution(evo_config, clock.clone())?;
        if let Some((ref desc, ref subs)) = resumed_objective {
            runtime.seed_goal(desc, subs);
        }

        // Pipeline, reflector, episodic memory
        let meta_runtime = DefaultMetaRuntime::new(Version::new(0, 1, 0), clock.clone());
        let pipeline = Arc::new(MorphogenesisPipeline::new(meta_runtime));
        let reflector = Reflector::new(clock.clone());
        let episodic_db_path = data_dir.join("episodic.db");
        let mut episodic_memory = EpisodicMemory::new(episodic_db_path, clock.clone());
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: data_dir.clone(),
            config: serde_json::Value::Null,
            bus: None,
        };
        episodic_memory.init(&ctx).await?;
        let episodic_memory = Arc::new(Mutex::new(episodic_memory));

        // Skills
        let skills_dir = fabric::paths::skills_dir();
        let mut skill_loader = SkillLoader::new(skills_dir);
        let loaded = skill_loader.load_all_enhanced();
        if loaded > 0 {
            info!(count = loaded, "Skills loaded at startup");
        }

        // Hooks
        let mut hook_registry = HookRegistry::new(clock.clone());
        audit_hook::register_audit_hook(&mut hook_registry);
        let hooks_dir = aletheon_dir.join("hooks");
        let hook_loader = corpus::hook::loader::HookLoader::new(hooks_dir);
        let user_hook_count = hook_loader.register_all(&mut hook_registry);
        if user_hook_count > 0 {
            info!(count = user_hook_count, "Loaded user hooks");
        }
        for plugin in skill_loader.plugins() {
            register_skill(plugin, &mut tools, &mut hook_registry);
        }
        let hook_registry = Arc::new(Mutex::new(hook_registry));

        // Cache-stable prefix
        let cm = core_memory.lock().await;
        let cached_prefix = PrefixBuilder::build(&config.system_prompt, skill_loader.skills(), &cm);
        drop(cm);
        info!(len = cached_prefix.len(), "Cache-stable prefix built");

        // CommunicationBus
        let bus = Arc::new(CommunicationBus::new());
        {
            let sf_module = crate::r#impl::engine::modules::self_field_module::SelfFieldModule::new(
                self_field.clone(),
            );
            let bus_clone = bus.clone();
            tokio::spawn(async move { sf_module.run(bus_clone).await });
        }
        {
            let mem_module = crate::r#impl::engine::modules::memory_module::MemoryModule::new(
                core_memory.clone(),
                Some(recall_memory.clone()),
            );
            let bus_clone = bus.clone();
            tokio::spawn(async move { mem_module.run(bus_clone).await });
        }
        let tools = Arc::new(Mutex::new(tools));
        {
            let body_module =
                crate::r#impl::engine::modules::body_module::BodyModule::new(tools.clone());
            let bus_clone = bus.clone();
            tokio::spawn(async move { body_module.run(bus_clone).await });
        }
        info!("CommunicationBus created with SelfField, Memory, and Body module handlers");

        // StormBreaker, CheckpointStore, SkillRouter, AgentLoader
        let storm_breaker = Arc::new(Mutex::new(StormBreaker::new(
            runtime_config_snapshot
                .agent_loop
                .storm_breaker_failure_threshold,
            runtime_config_snapshot
                .agent_loop
                .storm_breaker_success_threshold,
        )));
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
        let hooks_config = config.hooks.clone();

        // ModelRouter
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

        // AutoMemory
        let cheap_llm: Arc<dyn LlmProvider> =
            match model_router.create_provider(TaskType::AutoMemory) {
                Ok(provider) => {
                    info!(model = provider.name(), "AutoMemory using routed model");
                    Arc::from(provider)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ModelRouter AutoMemory failed, fallback");
                    Arc::from(registry.resolve_and_create("").expect("no LLM available"))
                }
            };
        let auto_memory = Arc::new(Mutex::new(AutoMemory::new(cheap_llm, core_memory.clone())));

        // Debug
        let debug_perf = Arc::new(PerfCounter::default());
        let debug_hook = Arc::new(tokio::sync::Mutex::new(DebugBusHook::new(
            EventFilter::default(),
        )));
        let debug_handler = Arc::new(DebugHandler::new(
            debug_hook,
            debug_perf.clone(),
            clock.clone(),
        ));

        // Session Gateway
        let param_registry = Arc::new(ParamRegistry::new());
        let gw_state = Arc::new(Mutex::new(SessionStateRef {
            iteration: 0,
            plan_mode: false,
            consecutive_errors: 0,
            circuit_breaker_status:
                cognit::harness::linear::circuit_breaker::CircuitBreakerStatus::Ok,
            tool_budget_remaining: runtime_config_snapshot.agent_loop.max_tool_calls,
            tool_budget_max: runtime_config_snapshot.agent_loop.max_tool_calls,
            recent_tools: Vec::new(),
            storm_breaker_failure_count: 0,
            goal_tracker: cognit::harness::linear::goal_tracker::GoalTracker::new(clock.clone()),
        }));
        let gw_started_at = std::time::Instant::now();
        let session_gateway = Arc::new(SessionGateway::new(
            param_registry.clone(),
            debug_handler.clone(),
            session_id.clone(),
            gw_state.clone(),
            initial_session.clone(),
            gw_started_at,
            runtime_config_snapshot,
            core_memory.clone(),
            recall_memory.clone(),
            self_field.clone(),
            llm.clone(),
        ));

        let ports = aletheon_kernel::service::ServicePorts::new()
            .with_agora(Arc::new(agora::AgoraRegistry::new()));
        let subsystems = Arc::new(crate::core::core_systems::CoreSystems {
            ports,
            runtime: Arc::new(Mutex::new(runtime)),
            self_field,
            reflector,
            memory: crate::core::MemoryGroup {
                memory_service: Arc::new(mnemosyne::DefaultMemoryService::new(
                    recall_memory.clone(),
                    fact_store.clone(),
                    core_memory.clone(),
                    episodic_memory.clone(),
                )),
                episodic_memory,
                recall_memory,
                core_memory,
                fact_store,
                auto_memory,
                objective_store,
            },
            security: crate::core::SecurityGroup {
                tool_runner,
                storm_breaker,
                approval_rx: Arc::new(Mutex::new(approval_rx)),
                pending_approvals: Arc::new(Mutex::new(HashMap::new())),
                session_approvals: Arc::new(Mutex::new(HashMap::new())),
            },
            corpus: crate::core::CorpusGroup {
                tools,
                skill_loader: Arc::new(Mutex::new(skill_loader)),
                skill_router,
                hook_registry,
                hooks_config,
            },
            session: crate::core::SessionGroup {
                default_session_id,
                session_created_at,
                cached_prefix: Arc::new(Mutex::new(cached_prefix)),
                memory_queue: Arc::new(Mutex::new(Vec::new())),
                context_window,
                config_prompt: config.system_prompt.clone(),
                data_dir,
            },
            pipeline,
            debug_handler,
            debug_perf,
            cancel_token: Arc::new(Mutex::new(None)),
            main_agent_process_id: Arc::new(Mutex::new(None)),
        });

        // Wire sub-agent execution runtime so spawned sub-agents run real
        // LLM + tool reasoning instead of the cancellation-wait stub.
        {
            let sub_agent_runtime = Arc::new(DaemonSubAgentRuntime {
                llm: llm.clone(),
                tools: subsystems.corpus.tools.clone(),
            });
            subsystems
                .runtime
                .lock()
                .await
                .sub_agent_spawner_mut()
                .with_runtime(sub_agent_runtime);
        }

        // Repoint the sub-agent spawner at the shared kernel tables so
        // sub-agents register in the same ProcessTable/OperationTable as the
        // main agent (Phase 2c: enables fork-on-spawn for process parents).
        {
            let pt = subsystems.ports.process_table.clone();
            let ot = subsystems.ports.operation_table.clone();
            subsystems
                .runtime
                .lock()
                .await
                .sub_agent_spawner_mut()
                .set_shared_tables(pt, ot);
        }

        // AgentTool — delegates to SubAgentSpawner for process tracking,
        // supervision, and cancellation, with inline LLM loop for execution.
        {
            let agents_dir = aletheon_dir.join("agents");
            let mut rt_agent_loader = AgentLoader::new();
            if agents_dir.exists() {
                let _ = rt_agent_loader.load_from_dir(&agents_dir);
            }
            let mut agent_defs: HashMap<String, corpus::tools::tools::agent_tool::AgentDefinition> =
                HashMap::new();
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
                let llm_for_agents: Arc<dyn LlmProvider> = llm.clone();
                let tools_for_agents = subsystems.corpus.tools.clone();
                let exec_for_agents = subsystems.runtime.clone();
                let main_slot = subsystems.main_agent_process_id.clone();

                let _clock_for_agents = clock.clone();
                let execute_fn: corpus::tools::tools::agent_tool::ExecuteSubAgentFn =
                    Arc::new(move |system_prompt, user_prompt, allowed_tools| {
                        let llm = llm_for_agents.clone();
                        let tools = tools_for_agents.clone();
                        let exec = exec_for_agents.clone();
                        let main_slot = main_slot.clone();
                        let sp = system_prompt;
                        let up = user_prompt;
                        let at = allowed_tools;
                        let clock = clock.clone();
                        Box::pin(async move {
                            // 1. Register tracked sub-agent with SubAgentSpawner.
                            let agent_id = {
                                let mut runtime = exec.lock().await;
                                let parent = *main_slot.lock().await;
                                let handle = runtime
                                    .sub_agent_spawner_mut()
                                    .spawn_tracked_with_parent(
                                        up.clone(),
                                        "agent-tool".into(),
                                        RestartPolicy::Never,
                                        parent,
                                    )
                                    .await?;
                                let id = handle.id.clone();
                                // Transition to Running so the agent is "active"
                                // in the process table.
                                let _ = runtime
                                    .sub_agent_spawner_mut()
                                    .transition(&id, SubAgentState::Running)
                                    .await;
                                id
                            };

                            // 2. Run the LLM loop (same as before, but with
                            //    SubAgentSpawner tracking for cancellation).
                            let result = {
                                let reg = tools.lock().await;
                                let agent_tool_defs: Vec<fabric::ToolDefinition> = reg
                                    .definitions()
                                    .into_iter()
                                    .filter(|d| at.contains(&d.name))
                                    .collect();
                                drop(reg);
                                let mut current_messages = vec![
                                    fabric::message::Message::system(&sp),
                                    fabric::message::Message::user(&up),
                                ];
                                #[allow(unused_assignments)]
                                let mut response_text = String::new();
                                let mut loop_result: Result<String, anyhow::Error> =
                                    Ok(String::new());
                                for _ in 0..20 {
                                    match llm.complete(&current_messages, &agent_tool_defs).await {
                                        Ok(response) => {
                                            let mut text_parts = Vec::new();
                                            let mut tool_calls = Vec::new();
                                            for block in &response.content {
                                                match block {
                                                    fabric::message::ContentBlock::Text {
                                                        text,
                                                    } => {
                                                        text_parts.push(text.clone());
                                                    }
                                                    fabric::message::ContentBlock::ToolUse {
                                                        id,
                                                        name,
                                                        input,
                                                    } => {
                                                        tool_calls.push((
                                                            id.clone(),
                                                            name.clone(),
                                                            input.clone(),
                                                        ));
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            if tool_calls.is_empty() {
                                                response_text = text_parts.join("\n");
                                                loop_result = Ok(response_text);
                                                break;
                                            }
                                            current_messages.push(fabric::message::Message {
                                                role: fabric::message::Role::Assistant,
                                                content: response.content.clone(),
                                            });
                                            for (cid, name, input) in tool_calls {
                                                let reg = tools.lock().await;
                                                let result = if let Some(tool) = reg.get(&name) {
                                                    let ctx = fabric::tool::ToolContext {
                                                        working_dir: std::env::current_dir()
                                                            .unwrap_or_default(),
                                                        session_id: "sub-agent".into(),
                                                        clock: clock.clone(),
                                                    };
                                                    tool.execute(input, &ctx).await
                                                } else {
                                                    fabric::tool::ToolResult {
                                                        content: format!("Unknown tool: {}", name),
                                                        is_error: true,
                                                        metadata:
                                                            fabric::tool::ToolResultMeta::default(),
                                                    }
                                                };
                                                drop(reg);
                                                current_messages.push(
                                                    fabric::message::Message::tool_result(
                                                        &cid,
                                                        &result.content,
                                                        result.is_error,
                                                    ),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            loop_result = Err(e);
                                            break;
                                        }
                                    }
                                }
                                loop_result
                            };

                            // 3. Update spawner state and clean up.
                            {
                                let mut runtime = exec.lock().await;
                                let spawner = runtime.sub_agent_spawner_mut();
                                match &result {
                                    Ok(_) => {
                                        let _ = spawner
                                            .transition(&agent_id, SubAgentState::Completed)
                                            .await;
                                    }
                                    Err(_) => {
                                        let _ = spawner
                                            .transition(&agent_id, SubAgentState::Failed)
                                            .await;
                                    }
                                }
                                let _ = spawner.destroy(&agent_id).await;
                            }

                            result.map_err(|e| anyhow::anyhow!("{e}"))
                        })
                    });
                let agent_tool = corpus::tools::tools::agent_tool::AgentTool::new(
                    agent_defs.clone(),
                    execute_fn,
                );
                if let Err(e) = subsystems
                    .corpus
                    .tools
                    .lock()
                    .await
                    .register(Arc::new(agent_tool))
                {
                    tracing::warn!(error = %e, "Failed to register AgentTool");
                } else {
                    info!(
                        agents = agent_defs.len(),
                        "Registered AgentTool with sub-agents"
                    );
                }
            }
        }

        let shared_notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>> = Arc::new(Mutex::new(None));

        let turn_orchestrator = Arc::new(crate::service::DaemonTurnOrchestrator::new(
            subsystems.clone(),
            sessions.clone(),
            session_gateway.clone(),
            llm.clone(),
            model_router.clone(),
            shared_notify_tx.clone(),
            active_connections.clone(),
            Instant::now(),
            Some(cancel_token.clone()),
        ));

        let handler = Self {
            subsystems,
            sessions,
            session_gateway,
            bus,
            llm,
            model_router,
            notify_tx: None,
            active_connections,
            started_at: Instant::now(),
            daemon_cancel_token: Some(cancel_token),
            turn_orchestrator,
        };

        // Register initial params
        {
            let data_dir_clone = handler.subsystems.session.data_dir.clone();
            let started_at = std::time::Instant::now();
            param_registry
                .declare(
                    "session.uptime_secs",
                    "session",
                    "Daemon uptime in seconds",
                    move || json!(started_at.elapsed().as_secs()),
                )
                .await;
            param_registry
                .declare(
                    "session.data_dir",
                    "session",
                    "Data directory path",
                    move || json!(data_dir_clone.to_string_lossy()),
                )
                .await;
            let model = config.model.clone();
            param_registry
                .declare("llm.model", "llm", "Current LLM model in use", move || {
                    json!(model)
                })
                .await;
            let provider_name = handler.llm.name().to_string();
            param_registry
                .declare(
                    "llm.provider",
                    "llm",
                    "Current LLM provider name",
                    move || json!(provider_name),
                )
                .await;
            let sandbox_pref = config.sandbox_preference.clone();
            param_registry
                .declare(
                    "sandbox.preference",
                    "sandbox",
                    "Current sandbox mode",
                    move || json!(sandbox_pref),
                )
                .await;
            param_registry
                .declare("session.rss_kb", "session", "Resident memory in KB", || {
                    let status = std::fs::read_to_string("/proc/self/status").ok();
                    let rss = status.and_then(|s| {
                        s.lines()
                            .find(|l| l.starts_with("VmRSS:"))
                            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                    });
                    json!(rss.unwrap_or(0))
                })
                .await;
            info!("Registered {} initial params", 6);
        }

        // Fire OnSessionStart hook
        {
            let hr = handler.subsystems.corpus.hook_registry.lock().await;
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
}
