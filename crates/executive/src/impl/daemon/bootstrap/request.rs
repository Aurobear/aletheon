//! Handler initialization, construction, and setup-related methods.

use super::super::model_router::{ModelRouter, TaskType};
use super::super::prefix_builder::PrefixBuilder;
use super::super::session_manager::SessionManager;
use super::super::DaemonConfig;
use crate::core::config::ExecutiveConfig;
use crate::core::evolution_coordinator::EvolutionConfig;
use crate::core::orchestrator::AletheonExecutive;
use crate::r#impl::daemon::handler::RequestHandler;
use crate::session::store::SessionStore;
use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use cognit::core::reflector::Reflector;
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::{create_default_executor, SandboxPreference};
use corpus::security::socket_approval::SocketApprovalGate;
use corpus::tools::tools::ToolRegistry;
use dasein::{SelfField, SelfFieldConfig};
use fabric::CanonicalEventBus;
use fabric::Clock;
use fabric::LlmProvider;
use fabric::Registry;
use fabric::Version;
use fabric::{Subsystem, SubsystemContext};
use metacog::DefaultMetaRuntime;
use mnemosyne::episodic::EpisodicMemory;
use mnemosyne::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};
use mnemosyne::CoreMemory;
use mnemosyne::RecallMemory;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::r#impl::channel::gmail::GmailGoalDraftCoordinator;
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::runtime::worktree_recovery::{WorktreeRecoveryConfig, WorktreeRecoveryService};
use crate::r#impl::runtime::{pi_rpc_environment_from_process, register_pi_runtime, PiRpcRuntime};
use crate::service::inference_port::{InferencePort, PortLlmProvider};
use crate::service::CapabilityService;
use corpus::hook::builtin::audit_hook;
use corpus::security::storm_breaker::StormBreaker;
use corpus::skill::plugin::register_skill;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;
use mnemosyne::FactStore;

use super::super::debug_handler::DebugHandler;
use crate::core::session_gateway::gateway::SessionStateRef;
use crate::core::session_gateway::{ParamRegistry, SessionGateway};
use fabric::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

use super::request_ports::{
    initialize_self_field, retention_admin_port, RequestFacadePorts, TurnRuntimeFacadePorts,
};

impl RequestHandler {
    pub async fn new(
        config: &DaemonConfig,
        inference: Arc<dyn InferencePort>,
        model_routing: crate::core::config::ModelRoutingConfig,
        model_aliases: HashMap<String, String>,
        goal_runtime: cognit::config::GoalRuntimeConfig,
        pi_runtime: cognit::config::PiRuntimeConfig,
        grok_hardening: crate::core::config::GrokHardeningConfig,
        sandbox_profiles: fabric::SandboxProfiles,
        evolution_enabled: bool,
        event_bus: Option<Arc<CanonicalEventBus>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::new(PortLlmProvider::new(
            inference.clone(),
            config.model.clone(),
        ));
        info!(provider = llm.name(), "LLM provider initialized");
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());

        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let data_dir_for_telegram = data_dir.clone();
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data dir: {}", data_dir.display()))?;
        let session_store = SessionStore::new(&data_dir)?;
        session_store.create_session(&session_id)?;

        info!(session_id = %session_id, "Created new session");

        // SelfField is constructed before the recurrent workspace registry.
        // Inject a once-bound reader now and bind it after registry creation.
        let conscious_context =
            Arc::new(crate::service::conscious_context_slot::ConsciousContextSlot::default());

        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            clock: Some(clock.clone()),
            conscious_context: Some(conscious_context.clone()),
            ..Default::default()
        };
        let mut self_field = SelfField::new(self_field_config);

        // SelfField owns the durable Dasein ledger. Restore its reducer version
        // before any turn can submit a transition against the persisted ledger.
        initialize_self_field(&mut self_field, &data_dir).await?;

        // Tier 2a: install the Runtime PermissionManager as the permission authority.
        {
            use crate::core::permission_manager::PermissionManager;
            self_field.set_permission_authority(std::sync::Arc::new(PermissionManager::new()));
        }
        let self_field = Arc::new(Mutex::new(self_field));

        // Wire DaseinEventBridge to canonical events if available.
        if let Some(ref bus) = event_bus {
            let sf = self_field.lock().await;
            sf.wire_dasein_event_bridge(bus).await?;
        }

        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db_path = data_dir.join("recall_memory.db");
        let recall_clock: Arc<dyn fabric::Clock> = Arc::new(SystemClock::new());
        let recall_memory = Arc::new(Mutex::new(RecallMemory::new(
            &recall_db_path,
            recall_clock,
        )?));

        // Every durable user-runtime store is rooted in the injected state
        // directory. Never rediscover HOME or a machine deployment path here.
        let aletheon_dir = data_dir.clone();
        std::fs::create_dir_all(&aletheon_dir)?;
        let production = config.deployment.mode == cognit::config::DeploymentMode::Production;
        let fact_root = data_dir.join("mnemosyne");
        std::fs::create_dir_all(&fact_root)?;
        let fact_store =
            FactStore::open(&fact_root.join("fact_store.db")).context("opening fact store")?;
        let fact_store = Arc::new(Mutex::new(fact_store));

        let objective_root = data_dir.join("goals");
        std::fs::create_dir_all(&objective_root)?;
        let objective_db_path = objective_root.join("objectives.db");
        let storage_quota = production
            .then(|| super::storage::deployment_storage_quota(&config.deployment))
            .transpose()?;
        let objective_store =
            ObjectiveStore::open(&objective_db_path).context("opening objective store")?;
        let objective_store = Arc::new(Mutex::new(objective_store));
        let apply_objective_store = Arc::new(std::sync::Mutex::new(
            ObjectiveStore::open(&objective_db_path).context("opening apply objective store")?,
        ));
        let approval_repository =
            crate::r#impl::approval::ApprovalRepository::open(&objective_db_path)
                .context("opening approval repository")?;
        let approval_repository = Arc::new(std::sync::Mutex::new(approval_repository));
        let gmail_goal_drafts = Arc::new(std::sync::Mutex::new(
            GmailGoalDraftCoordinator::open(&objective_db_path)
                .context("opening Gmail Goal draft coordinator")?,
        ));

        // M3: terminalize stale runtime calls before making their Goals ready.
        // Recovery records cancellation evidence and never invokes a runtime.
        {
            let store = objective_store.lock().await;
            let stale_attempts = store
                .recover_stale_attempts()
                .context("recovering stale goal attempts")?;
            if !stale_attempts.is_empty() {
                info!(
                    count = stale_attempts.len(),
                    "Cancelled stale goal attempts on start"
                );
            }

            // M2: clear stale process links, map legacy active objectives, and
            // preserve suspended/awaiting states.
            match store.recover_goals() {
                Ok(recovered) if !recovered.is_empty() => {
                    info!(
                        count = recovered.len(),
                        "Recovered persisted goals on start"
                    );
                    for g in &recovered {
                        info!(
                            goal_id = g.id.0,
                            state = %g.state,
                            version = g.version,
                            "Goal recovered"
                        );
                    }
                }
                Ok(_) => {
                    info!("No goals to recover");
                }
                Err(e) => {
                    warn!(error = %e, "Failed to recover goals on start");
                }
            }
        }

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

        // Clock for monotonic/wall timestamps. Created early so
        // all subsystems (including SessionManager) can route through it.

        // Reconcile retained coding worktrees before the Pi runtime can be
        // registered. Any unsafe cleanup or exhausted budget fails closed for
        // new coding work while leaving the rest of the daemon available.
        let pi_work_allowed = if pi_runtime.enabled {
            let recovery = objective_store
                .lock()
                .await
                .coding_job_recovery_records()
                .context("loading coding job recovery metadata")
                .and_then(|records| {
                    WorktreeRecoveryService::new(
                        WorktreeRecoveryConfig::production(pi_runtime.worktree_base.clone()),
                        records,
                        clock.clone(),
                    )
                })
                .and_then(|service| service.recover());
            match recovery {
                Ok(outcome) => {
                    if !outcome.quarantined.is_empty() {
                        warn!(
                            count = outcome.quarantined.len(),
                            "Unknown coding worktrees quarantined for manual review"
                        );
                    }
                    if let Some(reason) = &outcome.blocked_reason {
                        warn!(reason = %reason, "Pi coding work blocked by worktree recovery");
                    }
                    outcome.allow_new_pi_work
                }
                Err(error) => {
                    warn!(error = %error, "Pi coding work blocked: worktree recovery failed");
                    false
                }
            }
        } else {
            true
        };

        // Multi-session setup
        let context_window = llm.max_context_length();
        let initial_session =
            SessionManager::new(&data_dir, session_id.clone(), context_window, clock.clone())
                .await?;
        info!(
            context_window = context_window,
            "Session context window configured"
        );
        let initial_session = Arc::new(Mutex::new(initial_session));
        let mut sessions = HashMap::new();
        sessions.insert(session_id.clone(), initial_session.clone());
        let sessions = Arc::new(Mutex::new(sessions));
        let default_session_id = Arc::new(tokio::sync::Mutex::new(session_id.clone()));

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
            clock: clock.clone(),
        }));
        let _ = tools.register(Arc::new(CoreMemoryReplaceTool {
            memory: core_memory.clone(),
            clock: clock.clone(),
        }));
        let _ = tools.register(Arc::new(MemorySearchTool {
            recall: recall_memory.clone(),
            core_memory: core_memory.clone(),
            fact_store: Some(fact_store.clone()),
            clock: clock.clone(),
        }));
        let external_artifact_root = data_dir.join("external-artifacts");
        let (google, mut google_sync, google_sync_store, gmail_ingress) =
            match super::google::register_configured_google_read_tools(
                &mut tools,
                &objective_db_path,
                clock.clone(),
                &cancel_token,
                &external_artifact_root,
                storage_quota.clone(),
            ) {
                Ok(Some((integration, sync, store, gmail_ingress))) => {
                    (Some(integration), Some(sync), Some(store), gmail_ingress)
                }
                Ok(None) => (None, None, None, None),
                Err(error) => {
                    warn!(error = %error, "Google read integration disabled");
                    (None, None, None, None)
                }
            };
        if let (Some(handle), Some(store)) = (google_sync.as_mut(), google_sync_store) {
            let goal_store = Arc::new(std::sync::Mutex::new(
                ObjectiveStore::open(&objective_db_path)
                    .context("opening Google event Goal store")?,
            ));
            let mut goals = crate::r#impl::goal::GoalCoordinator::new(goal_store);
            if let Some(quota) = storage_quota.clone() {
                goals = goals.with_storage_quota(quota, 16 * 1024 * 1024);
            }
            let goals = Arc::new(goals);
            let notifications = Arc::new(
                crate::r#impl::google::DurableGoogleNotificationSink::open(
                    &data_dir.join("channels.db"),
                )
                .context("opening Google notification outbox")?,
            );
            let mut event_router = crate::r#impl::google::GoogleEventRouter::new_with_notifications(
                store.clone(),
                goals,
                notifications,
            );
            if let Some(ingress) = gmail_ingress {
                event_router = event_router.with_mail_ingress(ingress);
            }
            let sink = Arc::new(event_router);
            let dispatcher = crate::r#impl::google::GoogleEventDispatcher::new(
                store,
                sink,
                format!("daemon-dispatch-{}", uuid::Uuid::new_v4()),
                30_000,
            )?;
            let dispatch_clock = clock.clone();
            handle.spawn_supervised(move |cancel| async move {
                loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let now_ms = dispatch_clock.wall_now().0.max(0);
                    if let Err(error) = dispatcher.dispatch_due(now_ms, 100, &cancel).await {
                        warn!(error = %error, "Google event dispatch failed");
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                    }
                }
            });
        }

        // MCP servers. Keep the manager alive: gbrain recall/capture calls the
        // same authenticated connections after startup tool registration.
        let mut retained_mcp = None;
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
            if config.gbrain_memory.enabled {
                if mcp
                    .server_tools(&config.gbrain_memory.server_name)
                    .is_some()
                {
                    retained_mcp = Some(Arc::new(mcp));
                } else {
                    tracing::warn!(
                        server = %config.gbrain_memory.server_name,
                        "GBrain server unavailable; local memory remains active"
                    );
                }
            }
        }

        // Security
        let sandbox_pref = SandboxPreference::from_str(&config.sandbox_preference);
        let sandbox = create_default_executor(sandbox_pref, clock.clone());
        let audit_path = data_dir.join("audit.jsonl");
        let audit_logger = AuditLogger::new(audit_path)?;
        let (approval_gate, approval_rx) = SocketApprovalGate::new(clock.clone());
        let mut runner =
            ToolRunnerWithGuard::new(sandbox, audit_logger, clock.clone())
                .with_approval_gate(Arc::new(approval_gate));
        if grok_hardening.sandbox_profiles {
            runner = runner.with_sandbox_profiles(sandbox_profiles);
        }
        let tool_runner = Arc::new(Mutex::new(runner));

        let runtime_config = ExecutiveConfig {
            session_id: session_id.clone(),
            context_window_tokens: context_window,
            conscious_arbitration_mode: config.conscious_arbitration_mode,
            compaction_v2: grok_hardening.compaction_v2,
            ..Default::default()
        };
        let runtime_config_snapshot = runtime_config.clone();
        let cognitive_sessions: Arc<dyn crate::service::harness_factory::CognitiveSessionFactory> =
            Arc::new(
                crate::service::harness_factory::LinearCognitiveSessionFactory::new(
                    crate::service::harness_factory::harness_config_from_executive(
                        &runtime_config_snapshot,
                    ),
                    clock.clone(),
                ),
            );

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
        let meta_runtime = Arc::new(DefaultMetaRuntime::new(
            Version::new(0, 1, 0),
            clock.clone(),
        ));
        let metacog: Arc<dyn metacog::MetacogService> =
            Arc::new(metacog::DefaultMetacogService::with_state_path(
                meta_runtime,
                clock.clone(),
                data_dir.join("metacog-mutations.json"),
            )?);
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
        super::turn_runtime::register_configured_hooks(&mut hook_registry, &config.hooks);
        let runtime_extensions =
            super::extensions::index_runtime_extensions(&skill_loader, &hook_registry)?;
        let hook_registry = Arc::new(Mutex::new(hook_registry));

        // Cache-stable prefix
        let cached_prefix = PrefixBuilder::build(&config.system_prompt, skill_loader.skills());
        info!(len = cached_prefix.len(), "Cache-stable prefix built");

        let tools = Arc::new(Mutex::new(tools));

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

        // ModelRouter
        let model_router = Arc::new(ModelRouter::new(model_routing.clone(), inference.clone()));
        info!(
            default = %model_router.model_name_for(TaskType::General),
            multimodal = %model_router.model_name_for(TaskType::Multimodal),
            cheap = %model_router.model_name_for(TaskType::Simple),
            reasoning = %model_router.model_name_for(TaskType::Reasoning),
            "ModelRouter initialized"
        );

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
        let gw_started_at = clock.mono_now();
        let consolidation_repository =
            Arc::new(mnemosyne::consolidation::ConsolidationRepository::open(
                data_dir.join("memory_consolidation.db"),
            )?);
        let retention_repository = Arc::new(mnemosyne::RetentionRepository::open(
            data_dir.join("memory_retention.db"),
        )?);
        let local_memory: Arc<dyn mnemosyne::MemoryService> = Arc::new(
            mnemosyne::DefaultMemoryService::new(
                recall_memory.clone(),
                fact_store.clone(),
                core_memory.clone(),
                episodic_memory.clone(),
                clock.clone(),
            )
            .with_consolidation_repository(consolidation_repository)
            .with_retention_repository(retention_repository.clone()),
        );
        let gbrain_runtime = crate::r#impl::gbrain::build_gbrain_memory_runtime_with_retention(
            local_memory,
            retained_mcp,
            &config.gbrain_memory,
            clock.clone(),
            &cancel_token,
            Some(retention_repository.clone()),
        );
        let memory_admin_use_cases: Arc<
            dyn crate::service::request_use_cases::MemoryAdminUseCases,
        > = Arc::new(
            crate::service::request_use_cases::ProductionMemoryAdminUseCases::new(
                gbrain_runtime.memory_service.clone(),
                retention_admin_port(retention_repository),
                fabric::LOCAL_OWNER_PRINCIPAL.to_string(),
            ),
        );
        let gbrain_worker_task = gbrain_runtime.worker_task;
        let consolidation_cancel = cancel_token.clone();
        let consolidation_memory = gbrain_runtime.memory_service.clone();
        tokio::spawn(async move {
            crate::service::memory_consolidation_worker::MemoryConsolidationWorker::new(
                consolidation_memory,
            )
            .run(consolidation_cancel)
            .await;
        });

        let kernel = Arc::new(aletheon_kernel::KernelRuntime::with_clock(clock.clone()));
        let fact_use_cases: Arc<dyn mnemosyne::FactUseCases> =
            Arc::new(mnemosyne::DefaultFactUseCases::new(fact_store.clone()));
        let goal_use_cases: Arc<dyn crate::service::GoalUseCases> =
            Arc::new(crate::service::GoalService::new(objective_store.clone()));
        let runtime = Arc::new(Mutex::new(runtime));
        let admin_runtime = runtime.clone();
        let admin_tools = tools.clone();
        let skill_loader = Arc::new(Mutex::new(skill_loader));
        let admin_skill_loader = skill_loader.clone();
        let admin_hooks = hook_registry.clone();
        let cached_prefix = Arc::new(Mutex::new(cached_prefix));
        let admin_cached_prefix = cached_prefix.clone();
        let pending_approvals = crate::service::admin_service::PendingApprovals::default();
        let admin_pending_approvals = pending_approvals.clone();
        let session_approvals = crate::service::admin_service::ScopedApprovalCache::default();
        let admin_session_approvals = session_approvals.clone();
        let memory_queue = Arc::new(Mutex::new(Vec::new()));
        let dasein_handle = self_field
            .lock()
            .await
            .dasein_handle()
            .context("Dasein must be enabled for the recurrent conscious workspace")?;
        let conscious_registry = Arc::new(
            crate::service::conscious_workspace::ConsciousWorkspaceRegistry::production_with_mode(
                data_dir.join("conscious_workspace.db"),
                Arc::new(
                    crate::service::dasein_workspace_adapter::DaseinWorkspaceAdapter::new(
                        dasein_handle,
                        clock.clone(),
                    ),
                ),
                kernel.clone(),
                clock.clone(),
                gbrain_runtime.memory_service.clone(),
                skill_loader.clone(),
                config.conscious_arbitration_mode,
            )?,
        );
        conscious_context.bind(conscious_registry.clone())?;
        let context_source = Arc::new(crate::service::context_assembler::ProductionContextSource {
            cached_prefix: cached_prefix.clone(),
            skill_loader: skill_loader.clone(),
            skill_router: skill_router.clone(),
            conscious: conscious_context.clone(),
        });
        let context_assembler = Arc::new(crate::service::context_assembler::ContextAssembler::new(
            context_source,
        ));
        let memory_group = crate::core::MemoryGroup {
            memory_service: gbrain_runtime.memory_service,
            supplemental_memory_health: gbrain_runtime.health,
            episodic_memory,
            objective_store,
            approval_repository,
        };
        let security_group = crate::core::SecurityGroup {
            tool_runner,
            storm_breaker,
            approval_rx: Arc::new(Mutex::new(approval_rx)),
            pending_approvals,
            session_approvals,
        };
        let corpus_group = crate::core::CorpusGroup {
            tools,
            hook_registry,
        };
        let corpus_executor = Arc::new(corpus::CorpusToolExecutor::new(
            corpus_group.tools.clone(),
            security_group.tool_runner.clone(),
            clock.clone(),
        ));
        let corpus: Arc<dyn corpus::CorpusService> =
            Arc::new(corpus::DefaultCorpusService::from_runtime_with_extensions(
                corpus_group.tools.clone(),
                corpus_executor,
                corpus_group.hook_registry.clone(),
                runtime_extensions.catalog,
            ));
        let extension_decisions = super::extensions::activate_runtime_extensions(
            corpus.clone(),
            runtime_extensions.ids,
            runtime_extensions.capabilities,
            &data_dir,
            &session_id,
        )
        .await?;
        let granted_capabilities = Arc::new(tokio::sync::RwLock::new(
            corpus::discover_tool_extensions(&corpus_group.tools)
                .await?
                .into_iter()
                .flat_map(|entry| entry.capabilities)
                .collect(),
        ));
        let domains = crate::core::DomainPorts::new(
            Arc::new(agora::AgoraRegistry::new(kernel.clock())),
            metacog,
            corpus,
            cognitive_sessions,
        );
        let session_group = crate::core::SessionGroup {
            default_session_id: default_session_id.clone(),
            session_created_at: session_created_at.clone(),
            memory_queue,
            context_window,
            data_dir: data_dir.clone(),
        };
        let turn_token = Arc::new(Mutex::new(None));
        let main_agent_process_id = Arc::new(Mutex::new(None));
        let capability_resources =
            crate::r#impl::daemon::handler::tool_executor::CapabilityResources {
                kernel: kernel.clone(),
                corpus: domains.corpus(),
                capabilities: granted_capabilities.clone(),
                storm: security_group.storm_breaker.clone(),
                memory_queue: session_group.memory_queue.clone(),
                approvals: security_group.session_approvals.clone(),
                perf: debug_perf.clone(),
                self_field: self_field.clone(),
                extension_decisions,
            };
        let capability_service: Arc<dyn CapabilityService> = Arc::new(
            crate::r#impl::daemon::handler::tool_executor::ProductionCapabilityService::new(
                capability_resources.clone(),
            ),
        );
        let agent_runtimes =
            Arc::new(crate::service::agent_control::AgentRuntimeRegistry::default());
        let agent_profiles_for_tools;

        // Ordinary child Agents use one Cognit session runtime. Goal worker
        // and reviewer attempts remain explicit ProviderWorkerRuntime routes.
        {
            let definitions = corpus_group.tools.lock().await.definitions();
            let (profiles, tool_profiles) = super::runtime::load_agent_profiles(
                &aletheon_dir.join("agents"),
                inference.clone(),
                llm.clone(),
                &definitions,
                &runtime_config_snapshot,
            )?;
            agent_profiles_for_tools = tool_profiles;
            let native = Arc::new(crate::r#impl::runtime::NativeCognitRuntime::new(
                crate::r#impl::runtime::NativeCognitRuntimeResources {
                    sessions: domains.cognition(),
                    capabilities: capability_service.clone(),
                    profiles,
                    clock: clock.clone(),
                    conscious_actions: Some(conscious_registry.clone()),
                    conscious_candidates: Some(conscious_registry.clone()),
                    grok_hardening: grok_hardening.clone(),
                },
            ));
            agent_runtimes.register(
                crate::r#impl::runtime::NativeCognitRuntime::runtime_id(),
                native,
            )?;
        }

        // Goal worker/reviewer runtimes are opt-in and strictly alias-resolved.
        // Missing routes fail startup only when Goal execution is enabled.
        {
            let mut runtime = runtime.lock().await;
            let registered = super::runtime::register_goal_runtimes(
                runtime.compatibility_runtimes_mut(),
                &goal_runtime,
                inference.clone(),
                &model_aliases,
                corpus_group.tools.lock().await.definitions(),
                capability_service.clone(),
                clock.clone(),
            )?;
            if !registered.is_empty() {
                info!(runtime_ids = ?registered, "Goal runtimes registered");
            }
            for runtime_id in &registered {
                let compatibility = runtime.compatibility_runtimes().resolve(runtime_id)?;
                agent_runtimes.register(
                    runtime_id.clone(),
                    Arc::new(
                        crate::service::agent_control::CompatibilityRuntimeLauncher::new(
                            compatibility,
                        ),
                    ),
                )?;
            }
        }

        // Coding jobs are fail-closed: only a probed namespace backend may
        // register the stable `pi-coder` runtime ID.
        {
            let sandbox = corpus::security::sandbox::BubblewrapBackend::probe_async(clock.clone())
                .await
                .map(|backend| Arc::new(backend) as Arc<dyn fabric::sandbox::SandboxBackend>);
            let pi_rpc = if pi_work_allowed {
                match sandbox.clone() {
                    Some(sandbox) => PiRpcRuntime::prepare(
                        &pi_runtime,
                        sandbox,
                        clock.clone(),
                        pi_rpc_environment_from_process(),
                    )?,
                    None => None,
                }
            } else {
                None
            };
            let mut runtime = runtime.lock().await;
            let registered = pi_work_allowed
                && register_pi_runtime(
                    runtime.compatibility_runtimes_mut(),
                    &pi_runtime,
                    sandbox,
                    clock.clone(),
                )?;
            if registered {
                let runtime_id = crate::r#impl::runtime::PI_CODER_RUNTIME_ID;
                let compatibility = runtime
                    .compatibility_runtimes()
                    .resolve(&fabric::RuntimeId(runtime_id.into()))?;
                agent_runtimes.register(
                    fabric::RuntimeId(runtime_id.into()),
                    Arc::new(
                        crate::service::agent_control::CompatibilityRuntimeLauncher::new(
                            compatibility,
                        ),
                    ),
                )?;
                info!(runtime_id = "pi-coder", "Pi coding runtime registered");
            }
            if let Some(pi_rpc) = pi_rpc {
                agent_runtimes.register(PiRpcRuntime::runtime_id(), Arc::new(pi_rpc))?;
                info!(runtime_id = "pi-rpc", "Pi resident RPC runtime registered");
            }
        }

        let goal_runtime_registry = if goal_runtime.enabled {
            let runtime = runtime.lock().await;
            Some(Arc::new(runtime.compatibility_runtimes().clone()))
        } else {
            None
        };

        // Clone clock for later daemon services.
        let clock_2 = clock.clone();

        let agent_state_root = data_dir.join("agents");
        std::fs::create_dir_all(&agent_state_root)?;
        let agent_repository = Arc::new(
            crate::service::agent_control::SqliteAgentRunRepository::open(
                agent_state_root.join("agent_control.db"),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        );
        let canonical_event_spine = Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(
                data_dir.join("events.db"),
            )
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "canonical event spine unavailable; using process-local fallback");
                crate::r#impl::events::SqliteEventSpine::open(":memory:")
                    .expect("in-memory event spine")
            }),
        );
        let event_projections = Arc::new(
            crate::r#impl::events::DefaultEventProjectionSet::open(
                data_dir.join("event-projections.db"),
            )
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "event projections unavailable; using process-local fallback");
                crate::r#impl::events::DefaultEventProjectionSet::in_memory()
            }),
        );
        let agent_control_service = Arc::new(
            crate::service::agent_control::AgentControlService::new(
                kernel.clone(),
                clock.clone(),
                agent_repository.clone(),
                Arc::new(
                    crate::service::agent_control::BoundedAgentAdmission::with_budget(
                        config.agent_admission.clone(),
                        kernel.budget_controller(),
                    )
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                ),
                agent_runtimes,
            )
            .with_event_spine(canonical_event_spine.clone())
            .with_event_projections(event_projections.clone())
            .with_memory_vault(Arc::new(
                mnemosyne::AgentMemoryVault::open(agent_state_root.join("agent_memory.db"))
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?,
            )),
        );
        let agent_recovery = agent_control_service
            .reconcile_startup(&format!("daemon:{}", uuid::Uuid::new_v4()))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if !agent_recovery.ready() {
            anyhow::bail!(
                "Agent recovery left {} failed and {} unreconciled rows",
                agent_recovery.recovery_failed,
                agent_recovery.unreconciled
            );
        }
        info!(
            open = agent_recovery.open_rows,
            interrupted = agent_recovery.interrupted,
            resumed = agent_recovery.resumed,
            finalized = agent_recovery.finalized,
            "Agent restart recovery completed before spawn admission"
        );
        let agent_cleanup = crate::service::agent_control::AgentCleanupCoordinator::new(
            agent_repository.clone(),
            Arc::new(
                crate::r#impl::runtime::worktree_recovery::VerifiedAgentWorktreeReclaimer::default(
                ),
            ),
        )
        .reclaim_expired(clock.wall_now().0)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        info!(
            examined = agent_cleanup.examined,
            reclaimed = agent_cleanup.reclaimed,
            retained_unsafe = agent_cleanup.retained_unsafe,
            failures = agent_cleanup.failures,
            compacted = agent_cleanup.compacted_rows,
            "Agent terminal resource cleanup completed"
        );
        let agent_control: Arc<dyn fabric::AgentControlPort> = agent_control_service.clone();
        let agent_shutdown_cancel = cancel_token.clone();
        tokio::spawn(async move {
            agent_shutdown_cancel.cancelled().await;
            agent_control_service.shutdown().await;
        });

        super::runtime::register_agent_tools(
            corpus_group.tools.clone(),
            agent_control.clone(),
            agent_profiles_for_tools,
        )
        .await;
        *granted_capabilities.write().await = corpus::discover_tool_extensions(&corpus_group.tools)
            .await?
            .into_iter()
            .flat_map(|entry| entry.capabilities)
            .collect();

        let shared_notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>> = Arc::new(Mutex::new(None));
        let session_db = data_dir.join("sessions-v1.db");
        if let Some(parent) = session_db.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let canonical_store =
            crate::r#impl::session::canonical_store::CanonicalSessionStore::open(&session_db)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, path = %session_db.display(), "canonical session store unavailable; using process-local fallback");
                    crate::r#impl::session::canonical_store::CanonicalSessionStore::open(":memory:")
                        .expect("in-memory canonical session store")
                });
        let session_recovery =
            crate::r#impl::session::event_sourced_store::reconcile_committed_session_events(
                canonical_event_spine.as_ref(),
                event_projections.as_ref(),
                &canonical_store,
            )
            .await
            .context("reconcile committed Session events during daemon startup")?;
        info!(
            scanned = session_recovery.scanned,
            materialized = session_recovery.materialized,
            "Session event-spine recovery completed before turn admission"
        );
        let coordinator = Arc::new(
            crate::service::turn_coordinator::TurnCoordinator::with_event_spine_and_grok(
                kernel.clone(),
                Arc::new(canonical_store),
                canonical_event_spine.clone(),
                grok_hardening.clone(),
            )
            .with_event_projections(event_projections.clone()),
        );
        let session_service = Arc::new(crate::service::session_service::SessionService::new(
            coordinator.store(),
            coordinator.active_index(),
        ));
        if let Some(replay) = session_service
            .try_resume(&fabric::SessionId(session_id.clone()))
            .await?
        {
            initial_session
                .lock()
                .await
                .restore_messages(replay.messages);
        }
        let session_gateway = Arc::new(SessionGateway::new(
            param_registry.clone(),
            debug_handler.clone(),
            session_id.clone(),
            gw_state.clone(),
            initial_session.clone(),
            session_service.clone(),
            gw_started_at,
            runtime_config_snapshot.clone(),
            core_memory.clone(),
            recall_memory.clone(),
            self_field.clone(),
            llm.clone(),
            clock.clone(),
        ));
        let projection: Arc<dyn crate::service::post_turn_projection::PostTurnProjection> =
            Arc::new(
                crate::service::post_turn_projection::ProductionPostTurnProjection::new(
                    crate::service::post_turn_projection::PostTurnProjectionResources {
                        corpus: domains.corpus(),
                        executive: runtime.clone(),
                        evolution: domains.metacog(),
                    },
                ),
            );
        let turn_runtime_facades = TurnRuntimeFacadePorts::new(runtime.clone(), self_field.clone());
        let runtime_ports = Arc::new(super::turn_runtime::compose_turn_runtime(
            super::turn_runtime::TurnRuntimeResources {
                corpus: domains.corpus(),
                storm: security_group.storm_breaker.clone(),
                model_router: model_router.clone(),
                default_llm: llm.clone(),
                self_policy: turn_runtime_facades.self_policy,
                approval_rx: security_group.approval_rx.clone(),
                pending_approvals: security_group.pending_approvals.clone(),
                capabilities: capability_resources,
                admission: kernel.admission(),
                sessions: sessions.clone(),
                default_session_id: session_group.default_session_id.clone(),
                session_created_at: session_group.session_created_at.clone(),
                data_dir: session_group.data_dir.clone(),
                context_window: session_group.context_window,
                clock: clock.clone(),
                memory: memory_group.memory_service.clone(),
                config: turn_runtime_facades.config,
                performance: debug_perf.clone(),
            },
        ));
        let pipeline = Arc::new(crate::service::TurnPipeline::new(
            crate::service::turn_pipeline::TurnPipelineResources {
                session_gateway: session_gateway.clone(),
                notify: shared_notify_tx.clone(),
                clock: clock.clone(),
                agora: Some(domains.agora()),
                kernel: kernel.clone(),
                current_scope: Arc::new(Mutex::new(None)),
                daemon_cancel: Some(cancel_token.clone()),
                context: context_assembler,
                canonical_sessions: session_service.clone(),
                projection,
                runtime: runtime_ports,
                cognitive_sessions: domains.cognition(),
                conscious_core: Some(conscious_registry),
            },
        ));
        let turn_orchestrator = Arc::new(crate::service::DaemonTurnOrchestrator::new(
            crate::service::daemon_turn::DaemonTurnResources {
                kernel: kernel.clone(),
                notify: shared_notify_tx.clone(),
                default_session_id: session_group.default_session_id.clone(),
                main_agent_process_id: main_agent_process_id.clone(),
                turn_token: turn_token.clone(),
                pipeline,
                coordinator,
                session_service,
                grok_hardening: grok_hardening.clone(),
            },
        ));

        let approved_apply = if pi_runtime.enabled && pi_work_allowed {
            Some(Arc::new(
                crate::r#impl::approval::ApplyCoordinator::new(
                    apply_objective_store,
                    memory_group.approval_repository.clone(),
                    kernel.clone(),
                    clock.clone(),
                    crate::r#impl::approval::ApplyCoordinatorConfig {
                        worktree_base: pi_runtime.worktree_base.clone(),
                        timeout: std::time::Duration::from_secs(60),
                    },
                    Arc::new(crate::r#impl::approval::GitManagedWorktreeCleaner),
                )?
                .with_memory_projection(
                    crate::r#impl::memory_projection::MemoryProjection::new(
                        canonical_event_spine.clone(),
                        event_projections.clone(),
                    ),
                ),
            ))
        } else {
            None
        };

        // Clone these before they are moved into the handler struct
        // so they are available for Telegram channel initialization.
        let _turn_orch_for_telegram = turn_orchestrator.clone();
        let _cancel_for_telegram = cancel_token.clone();
        let (goal_progress_tx, goal_progress_rx) = mpsc::channel(64);
        let goal_worker_task = if let Some(runtime_registry) = goal_runtime_registry {
            let worker_route = goal_runtime
                .worker
                .as_ref()
                .context("missing Goal worker route")?;
            let reviewer_route = goal_runtime
                .reviewer
                .as_ref()
                .context("missing Goal reviewer route")?;
            let worker = crate::r#impl::goal::GoalWorker::new(
                Arc::new(std::sync::Mutex::new(ObjectiveStore::open(
                    &objective_db_path,
                )?)),
                runtime_registry,
                fabric::RuntimeId(worker_route.runtime_id.clone()),
                fabric::RuntimeId(reviewer_route.runtime_id.clone()),
                goal_progress_tx,
            );
            let worker = match storage_quota.clone() {
                Some(quota) => worker.with_storage_quota(quota, 16 * 1024 * 1024),
                None => worker,
            };
            let worker_cancel = cancel_token.clone();
            Some(tokio::spawn(worker.run(worker_cancel)))
        } else {
            drop(goal_progress_tx);
            None
        };
        let goal_worker_enabled = goal_worker_task.is_some();
        let goal_worker_task = goal_worker_task.map(|task| Arc::new(Mutex::new(Some(task))));
        let google_sync = google_sync.map(|handle| Arc::new(Mutex::new(Some(handle))));
        let gbrain_worker_task = gbrain_worker_task.map(|task| Arc::new(Mutex::new(Some(task))));
        let self_field_shutdown = Arc::new(Mutex::new(Some(self_field.clone())));

        let approval_use_cases: Arc<dyn crate::service::ApprovalUseCases> =
            Arc::new(crate::service::ApprovalService::new(
                memory_group.approval_repository.clone(),
                approved_apply.clone(),
                clock.clone(),
                main_agent_process_id.clone(),
            ));
        let admin_use_cases: Arc<dyn crate::service::AdminUseCases> = Arc::new(
            crate::service::AdminService::new(crate::service::admin_service::AdminResources {
                orchestrator: admin_runtime,
                skills: Arc::new(crate::service::admin_service::DefaultSkillAdmin::new(
                    admin_skill_loader,
                    admin_cached_prefix,
                    config.system_prompt.clone(),
                )),
                tool_catalog: Arc::new(move || {
                    let tools = admin_tools.clone();
                    Box::pin(async move { tools.lock().await.definitions() })
                }),
                hook_catalog: Arc::new(move || {
                    let hooks = admin_hooks.clone();
                    Box::pin(async move {
                        hooks
                            .lock()
                            .await
                            .list()
                            .into_iter()
                            .map(|hook| crate::service::admin_service::HookDescriptor {
                                name: hook.name.clone(),
                                source: hook.source.clone(),
                                point: format!("{:?}", hook.point),
                                priority: hook.priority,
                                script_path: hook.script_path.clone(),
                            })
                            .collect()
                    })
                }),
                pending_approvals: admin_pending_approvals,
                session_approvals: admin_session_approvals,
                daemon_cancel: cancel_token.clone(),
                google_sync: google_sync.clone(),
                gbrain_worker: gbrain_worker_task.clone(),
                goal_worker: goal_worker_task.clone(),
                runtime_shutdown: Arc::new(move || {
                    let self_field_shutdown = self_field_shutdown.clone();
                    Box::pin(async move {
                        let mut pending = self_field_shutdown.lock().await;
                        let Some(self_field) = pending.as_ref() else {
                            return Ok(());
                        };
                        self_field.lock().await.shutdown().await.map_err(|error| {
                            crate::service::admin_service::AdminServiceError::Operation(
                                error.to_string(),
                            )
                        })?;
                        pending.take();
                        Ok(())
                    })
                }),
                memory_admin: Some(memory_admin_use_cases),
                agent_runs: Some(agent_repository),
            }),
        );
        let legacy_sessions: Arc<
            dyn crate::service::legacy_session_service::LegacySessionUseCases,
        > = Arc::new(
            crate::service::legacy_session_service::LegacySessionService::new(
                crate::service::legacy_session_service::LegacySessionResources {
                    registry: sessions.clone(),
                    created_at: session_created_at,
                    data_dir: data_dir.clone(),
                    context_window,
                    clock: clock.clone(),
                    llm: llm.clone(),
                    canonical: turn_orchestrator.session_service.clone(),
                },
            ),
        );
        let started_at = clock_2.mono_now();
        let health_registry = Arc::new(crate::r#impl::health::HealthRegistry::production_ready());
        let telegram_task = Arc::new(Mutex::new(None));
        let request_facades = RequestFacadePorts::new(
            runtime.clone(),
            memory_group.episodic_memory.clone(),
            self_field.clone(),
            memory_group.supplemental_memory_health.clone(),
            grok_hardening.clone(),
        );
        let session_lifecycle: Arc<
            dyn crate::service::request_use_cases::SessionLifecycleUseCases,
        > = Arc::new(
            crate::service::request_use_cases::ProductionSessionLifecycle::new(
                domains.corpus(),
                security_group.session_approvals.clone(),
                turn_token.clone(),
            ),
        );
        let health_use_cases: Arc<dyn crate::service::request_use_cases::HealthUseCases> = Arc::new(
            crate::service::request_use_cases::ProductionHealthUseCases::new(
                crate::service::request_use_cases::ProductionHealthResources {
                    runtime_port: request_facades.runtime_port.clone(),
                    reflections: request_facades.reflections.clone(),
                    self_status: request_facades.self_status,
                    supplemental: request_facades.supplemental,
                    data_root: data_dir.clone(),
                    registry: health_registry,
                    clock: clock.clone(),
                    started_at,
                    active_connections: active_connections.clone(),
                    daemon_cancel: cancel_token.clone(),
                    telegram_task: telegram_task.clone(),
                    google_sync: google_sync.clone(),
                    goal_worker: goal_worker_task.clone(),
                    agent_recovery: agent_recovery.clone(),
                },
            ),
        );
        let reflection_use_cases: Arc<dyn crate::service::request_use_cases::ReflectionUseCases> =
            Arc::new(
                crate::service::request_use_cases::ProductionReflectionUseCases::new(
                    request_facades.runtime_port.clone(),
                    request_facades.reflections,
                    domains.metacog(),
                    super::request_ports::reflection_engine_port(reflector.clone()),
                ),
            );
        let google_use_cases: Arc<dyn crate::service::request_use_cases::GoogleUseCases> = Arc::new(
            crate::service::request_use_cases::ProductionGoogleUseCases::new(
                google.clone(),
                domains.corpus(),
                granted_capabilities.clone(),
                clock.clone(),
            ),
        );
        let workflow_use_cases: Arc<dyn crate::service::request_use_cases::WorkflowUseCases> =
            Arc::new(
                crate::service::request_use_cases::ProductionWorkflowUseCases::new(
                    crate::r#impl::orchestration::store::WorkflowStore::default_dir(),
                ),
            );
        let turn_use_cases: Arc<dyn crate::service::request_use_cases::TurnUseCases> = Arc::new(
            crate::service::request_use_cases::ProductionTurnUseCases::new(
                turn_orchestrator.clone(),
                request_facades.runtime_port,
                turn_token.clone(),
                turn_orchestrator.session_service.clone(),
            ),
        );
        let debug_use_cases: Arc<dyn crate::service::request_use_cases::DebugUseCases> = Arc::new(
            crate::service::request_use_cases::ProductionDebugUseCases(debug_handler.clone()),
        );
        let transport_ports = Arc::new(crate::r#impl::daemon::handler::ports::TransportPorts {
            corpus: domains.corpus(),
            capabilities_grant: corpus::ExtensionGrant {
                grant_id: "embedded-mcp".into(),
                principal: fabric::PrincipalId("embedded-mcp".into()),
                session_id: "embedded-mcp".into(),
                agent_id: None,
                capabilities: granted_capabilities.read().await.clone(),
                resources: fabric::CapabilityScope::default(),
            },
            capabilities: capability_service,
            clock: clock.clone(),
        });
        let handler_ports = Arc::new(crate::r#impl::daemon::handler::ports::HandlerPorts::new(
            fact_use_cases,
            goal_use_cases,
            approval_use_cases,
            admin_use_cases,
            legacy_sessions,
            session_lifecycle,
            health_use_cases,
            reflection_use_cases,
            google_use_cases,
            workflow_use_cases,
            turn_use_cases,
            debug_use_cases,
            session_gateway,
            transport_ports,
        ));
        let composition = super::DaemonComposition {
            request: handler_ports,
            active_connections,
            thread_authority: Arc::new(
                crate::service::thread_authority::ThreadAuthorityStore::persistent(
                    data_dir.join("thread-authority"),
                ),
            ),
            grok_hardening,
        };
        let handler = composition.into_handler();

        // Register initial params
        {
            let data_dir_clone = data_dir.clone();
            let started_at = clock_2.mono_now();
            param_registry
                .declare(
                    "session.uptime_secs",
                    "session",
                    "Daemon uptime in seconds",
                    move || {
                        let elapsed_ms = clock_2.mono_now().0.saturating_sub(started_at.0);
                        json!(elapsed_ms / 1000)
                    },
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
            let provider_name = llm.name().to_string();
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
        handler
            .ports
            .session_lifecycle
            .start(session_id.clone(), false)
            .await;

        // -- Telegram channel initialization (M1) -------------------------------
        let telegram_cfg = &config.telegram;
        if telegram_cfg.enabled {
            info!("Telegram channel enabled — initializing owner-only control channel");
            let jh = super::channels::init_telegram_channel(
                telegram_cfg,
                data_dir_for_telegram,
                _turn_orch_for_telegram,
                memory_group.objective_store.clone(),
                memory_group.approval_repository.clone(),
                gmail_goal_drafts,
                approved_apply,
                google.clone(),
                _cancel_for_telegram,
                goal_worker_enabled.then_some(goal_progress_rx),
            );
            *telegram_task.lock().await = Some(Arc::new(jh));
        } else {
            info!("Telegram channel disabled");
        }

        Ok(handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::dasein::{OutcomeStatus, SelfVersion};

    #[tokio::test]
    async fn daemon_self_field_bootstrap_replays_before_new_transitions() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("self_field.db");

        let mut first = SelfField::new(SelfFieldConfig {
            db_path: Some(database.clone()),
            clock: Some(Arc::new(aletheon_kernel::chronos::TestClock::new(100, 0))),
            ..Default::default()
        });
        initialize_self_field(&mut first, root.path())
            .await
            .unwrap();
        first
            .dasein()
            .unwrap()
            .record_outcome("first boot", OutcomeStatus::Succeeded, "daemon-bootstrap")
            .await
            .unwrap();
        first.shutdown().await.unwrap();

        let mut restarted = SelfField::new(SelfFieldConfig {
            db_path: Some(database),
            clock: Some(Arc::new(aletheon_kernel::chronos::TestClock::new(5_100, 0))),
            ..Default::default()
        });
        initialize_self_field(&mut restarted, root.path())
            .await
            .unwrap();
        assert!(matches!(
            restarted.health().await,
            fabric::SubsystemHealth::Healthy
        ));
        let dasein = restarted.dasein_handle().unwrap();
        assert_eq!(dasein.self_version().await, SelfVersion(2));
        let receipt = dasein
            .record_outcome(
                "first turn after restart",
                OutcomeStatus::Succeeded,
                "daemon-bootstrap",
            )
            .await
            .unwrap();
        assert_eq!(receipt.previous_version, SelfVersion(2));
        assert_eq!(receipt.current_version, SelfVersion(3));
        restarted.shutdown().await.unwrap();
    }
}
