//! Handler initialization, construction, and setup-related methods.

use super::super::model_router::{ModelRouter, TaskType};
use crate::composition::prefix_builder::PrefixBuilder;
use super::super::DaemonConfig;
use crate::composition::config::ExecutiveConfig;
use crate::core::evolution_coordinator::EvolutionConfig;
use crate::core::orchestrator::AletheonExecutive;
use crate::host::daemon::handler::RequestHandler;
use crate::adapters::session::store::SessionStore;
use anyhow::Context;
use kernel::chronos::SystemClock;

use super::approval_gate::{bootstrap_workspace_trust_resolver, DurableSocketApprovalGate};
use cognit::core::reflector::Reflector;
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::{create_executor_with_front_backend, SandboxPreference};
use corpus::security::socket_approval::SocketApprovalGate;
use dasein::{SelfField, SelfFieldConfig};
use fabric::CanonicalEventBus;
use fabric::Clock;
use fabric::Registry;
use fabric::Version;
use fabric::{Subsystem, SubsystemContext};
use metacog::DefaultMetaRuntime;
use mnemosyne::runtime::EpisodicMemory;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::application::inference_port::InferencePort;
use crate::application::CapabilityService;
use crate::adapters::channel::gmail::GmailGoalDraftCoordinator;
use crate::application::goal::ObjectiveStore;
use crate::adapters::runtime::worktree_recovery::{WorktreeRecoveryConfig, WorktreeRecoveryService};
use crate::adapters::runtime::{pi_rpc_environment_from_process, register_pi_runtime, PiRpcRuntime};
use corpus::hook::builtin::audit_hook;
use corpus::security::storm_breaker::StormBreaker;
use corpus::skill::plugin::register_skill;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;

use super::super::debug_handler::DebugHandler;
use crate::core::session_gateway::gateway::SessionStateRef;
use crate::core::session_gateway::ParamRegistry;
use fabric::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

use super::request_ports::{
    admin_runtime_port, initialize_self_field, retention_admin_port, RequestFacadePorts,
};

impl RequestHandler {
    pub async fn new(
        config: &DaemonConfig,
        inference: Arc<dyn InferencePort>,
        model_routing: crate::composition::config::ModelRoutingConfig,
        model_aliases: HashMap<String, String>,
        goal_runtime: cognit::config::GoalRuntimeConfig,
        pi_runtime: crate::composition::config::CodingRuntimeConfig,
        grok_hardening: crate::composition::config::GrokHardeningConfig,
        sandbox_profiles: fabric::SandboxProfiles,
        network_policy: fabric::network_policy::NetworkPolicy,
        agent_profiles: crate::composition::config::AgentProfilesConfig,
        evolution_enabled: bool,
        event_bus: Option<Arc<CanonicalEventBus>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<Self> {
        let llm = super::inference::compose(super::inference::InferenceCompositionInput {
            port: inference.clone(),
            model_spec: config.model.clone(),
        })
        .provider;
        info!(provider = llm.name(), "LLM provider initialized");
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());

        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let data_dir_for_telegram = data_dir.clone();
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data dir: {}", data_dir.display()))?;
        crate::application::durable_write::configure_writer_health(&data_dir);
        let session_store = SessionStore::new(&data_dir)?;
        session_store.create_session(&session_id)?;

        info!(session_id = %session_id, "Created new session");

        // SelfField is constructed before the recurrent workspace registry.
        // Inject a once-bound reader now and bind it after registry creation.
        let conscious_context =
            Arc::new(crate::application::conscious_context_slot::ConsciousContextSlot::default());

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

        let memory = super::memory::compose(super::memory::MemoryCompositionInput {
            data_dir: &data_dir,
            clock: clock.clone(),
        })?;

        // Every durable user-runtime store is rooted in the injected state
        // directory. Never rediscover HOME or a machine deployment path here.
        let aletheon_dir = data_dir.clone();
        std::fs::create_dir_all(&aletheon_dir)?;
        let production = config.deployment.mode == cognit::config::DeploymentMode::Production;
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
            crate::application::approval::ApprovalRepository::open(&objective_db_path)
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
        let sessions_composition =
            super::sessions::compose(super::sessions::SessionCompositionInput {
                data_dir: &data_dir,
                session_id: session_id.clone(),
                context_window,
                clock: clock.clone(),
            })
            .await?;
        info!(
            context_window = context_window,
            "Session context window configured"
        );
        let initial_session = sessions_composition.initial;
        let sessions = sessions_composition.registry;
        let default_session_id = sessions_composition.default_id;
        let session_created_at = sessions_composition.created_at;
        let active_connections = Arc::new(AtomicUsize::new(0));

        // Register tools
        let search_config = config.integrations.search.as_ref().map(|search| {
            corpus::tools::tools::web_search::WebSearchConfig::new(
                search.api_url.clone(),
                search.api_key.expose().to_owned(),
            )
        });
        let tool_composition = super::tools::compose(super::tools::ToolCompositionInput {
            network_policy,
            search: search_config,
            stores: memory,
            clock: clock.clone(),
        });
        let mut tools = tool_composition.registry;
        let core_memory = tool_composition.stores.core;
        let recall_memory = tool_composition.stores.recall;
        let fact_store = tool_composition.stores.facts;
        let external_artifact_root = data_dir.join("external-artifacts");
        let google_composition =
            super::integrations::compose_google(super::integrations::GoogleCompositionInput {
                tools: &mut tools,
                objective_db_path: &objective_db_path,
                clock: clock.clone(),
                cancel: &cancel_token,
                artifact_root: &external_artifact_root,
                storage_quota: storage_quota.clone(),
                config: config.integrations.google.as_ref(),
            });
        let google = google_composition.integration;
        let mut external_sync = google_composition.sync;
        let external_sync_store = google_composition.sync_store;
        let gmail_ingress = google_composition.gmail_ingress;
        if let (Some(handle), Some(store)) = (external_sync.as_mut(), external_sync_store) {
            let goal_store = Arc::new(std::sync::Mutex::new(
                ObjectiveStore::open(&objective_db_path)
                    .context("opening Google event Goal store")?,
            ));
            let mut goals = crate::application::goal::GoalCoordinator::new(goal_store);
            if let Some(quota) = storage_quota.clone() {
                goals = goals.with_storage_quota(quota, 16 * 1024 * 1024);
            }
            let goals = Arc::new(goals);
            let notifications = Arc::new(
                crate::adapters::google::DurableGoogleNotificationSink::open(
                    &data_dir.join("channels.db"),
                )
                .context("opening Google notification outbox")?,
            );
            let mut event_router = crate::adapters::google::GoogleEventRouter::new_with_notifications(
                store.clone(),
                goals,
                notifications,
            );
            if let Some(ingress) = gmail_ingress {
                event_router = event_router.with_mail_ingress(Arc::new(
                    crate::adapters::channel::handlers::gmail_ingest::ExternalEventIngestHandler::new(
                        ingress,
                    ),
                ));
            }
            let sink = Arc::new(event_router);
            let dispatcher = crate::adapters::google::GoogleEventDispatcher::new(
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

        // One approval gate is shared by guarded tools and MCP elicitation.
        let (socket_approval_gate, approval_rx) = SocketApprovalGate::new(clock.clone());
        let approval_gate: Arc<dyn corpus::security::approval::ApprovalGate> =
            Arc::new(DurableSocketApprovalGate {
                socket: Arc::new(socket_approval_gate),
                repository: approval_repository.clone(),
                clock: clock.clone(),
            });

        // MCP servers. Keep the manager alive: gbrain recall/capture calls the
        // same authenticated connections after startup tool registration.
        let (mcp_registry_tx, mut mcp_registry_rx) = tokio::sync::mpsc::channel::<String>(16);
        let mut mcp_registration_ids = Vec::new();
        let retained_mcp = {
            let mcp_config = corpus::tools::mcp::config::McpConfig {
                servers: config.mcp_servers.clone(),
                ..Default::default()
            };
            let mut mcp = corpus::tools::mcp::manager::McpManager::new(mcp_config);
            mcp.set_registry_change_sender(mcp_registry_tx);
            if let Err(e) = mcp.connect_all().await {
                tracing::warn!(error = %e, "MCP connect_all failed; continuing without MCP tools");
            }
            mcp.set_elicitation_approval_gate(approval_gate.clone());
            let mcp_count = mcp.connected_count();
            if mcp_count > 0 {
                info!(servers = mcp_count, "MCP servers connected");
            }
            for wrapper in mcp.tool_wrappers() {
                let name = wrapper.name().to_string();
                match tools.register(Arc::from(wrapper)) {
                    Err(e) => {
                        tracing::warn!(tool = %name, error = %e, "skip MCP tool (name clash?)");
                    }
                    Ok(id) => {
                        mcp_registration_ids.push(id);
                        info!(tool = %name, "Registered MCP tool");
                    }
                }
            }
            for wrapper in mcp.resource_wrappers() {
                let name = wrapper.name().to_string();
                match tools.register(Arc::from(wrapper)) {
                    Err(e) => {
                        tracing::warn!(tool = %name, error = %e, "skip MCP resource (name clash?)")
                    }
                    Ok(id) => {
                        mcp_registration_ids.push(id);
                        info!(tool = %name, "Registered MCP resource");
                    }
                }
            }
            if config.supplemental_memory.enabled
                && mcp
                    .server_tools(&config.supplemental_memory.server_name)
                    .is_none()
            {
                tracing::warn!(
                    server = %config.supplemental_memory.server_name,
                    "GBrain server unavailable; local memory remains active"
                );
            }
            Some(Arc::new(mcp))
        };

        // Security
        let sandbox_pref = SandboxPreference::from_str(&config.sandbox_preference);
        let mut structured_exec_backend: Option<Arc<dyn corpus::security::StructuredToolSandbox>> =
            None;
        let exec_backend: Option<Box<dyn fabric::SandboxBackend>> = if grok_hardening.execd {
            let binary_path = std::env::var_os("ALETHEON_EXECD_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|path| path.parent().map(|parent| parent.join("execd")))
                        .unwrap_or_else(|| std::path::PathBuf::from("execd"))
                });
            let workspace = std::path::PathBuf::from(&config.working_dir)
                .canonicalize()
                .context("canonicalize execd workspace root")?;
            let backend = crate::adapters::channel::execd_client::ExecdSandboxBackend::new(
                crate::adapters::channel::execd_client::ExecdConfig {
                    binary_path: binary_path.to_string_lossy().into_owned(),
                    shared_secret: format!(
                        "{}{}",
                        uuid::Uuid::new_v4().simple(),
                        uuid::Uuid::new_v4().simple()
                    ),
                    startup_timeout: std::time::Duration::from_secs(5),
                    request_timeout: std::time::Duration::from_secs(30),
                    workspace_roots: vec![workspace],
                },
            );
            structured_exec_backend = Some(Arc::new(backend.clone()));
            Some(Box::new(backend))
        } else {
            None
        };
        let sandbox = create_executor_with_front_backend(sandbox_pref, clock.clone(), exec_backend);
        let audit_path = data_dir.join("audit.jsonl");
        let audit_logger = AuditLogger::new(audit_path)?;
        let mut runner = ToolRunnerWithGuard::new(sandbox, audit_logger, clock.clone())
            .with_approval_gate(approval_gate);
        if let Some(structured) = structured_exec_backend {
            runner = runner.with_structured_sandbox(structured);
        }
        if grok_hardening.sandbox_profiles {
            runner = runner.with_sandbox_profiles(sandbox_profiles);
        }
        if let Some(bus) = event_bus.as_ref() {
            runner = runner.with_event_bus(bus.clone());
        }
        let tool_runner = Arc::new(Mutex::new(runner));

        let runtime_config = ExecutiveConfig {
            session_id: session_id.clone(),
            context_window_tokens: context_window,
            conscious_arbitration_mode: config.conscious_arbitration_mode,
            compaction_v2: grok_hardening.compaction_v2,
            streaming_tools: grok_hardening.streaming_tools,
            // Wave 0: honor configured agent iteration cap (0 = unlimited)
            // instead of the hardcoded Default (50).
            max_iterations: config.agent_max_iterations,
            harness_kind: config.harness_kind,
            ..Default::default()
        };
        let runtime_config_snapshot = runtime_config.clone();
        tracing::info!(
            harness = crate::application::harness_factory::selected_harness_kind(
                runtime_config_snapshot.harness_kind
            ),
            "cognitive harness selected from config"
        );
        let cognitive_sessions: Arc<
            dyn crate::application::harness_factory::CognitiveSessionFactory,
        > = Arc::new(
            crate::application::harness_factory::LinearCognitiveSessionFactory::new(
                crate::application::harness_factory::harness_config_from_executive(
                    &runtime_config_snapshot,
                ),
                clock.clone(),
            )
            .with_evicted_memory(recall_memory.clone()),
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
        let mut hook_registry = HookRegistry::new(clock.clone()).with_event_bus(event_bus.clone());
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
        if let Some(mcp) = retained_mcp.clone() {
            let registry = tools.clone();
            let registrations = Arc::new(Mutex::new(mcp_registration_ids));
            let cancel = cancel_token.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        changed = mcp_registry_rx.recv() => {
                            let Some(server) = changed else { break };
                            let mut registry = registry.lock().await;
                            let mut ids = registrations.lock().await;
                            for id in ids.drain(..) {
                                let _ = registry.unregister(id);
                            }
                            for wrapper in mcp.tool_wrappers().into_iter().chain(mcp.resource_wrappers()) {
                                let name = wrapper.name().to_string();
                                match registry.register(Arc::from(wrapper)) {
                                    Ok(id) => ids.push(id),
                                    Err(error) => tracing::warn!(%server, tool = %name, %error, "failed to refresh MCP registry entry"),
                                }
                            }
                            tracing::info!(%server, count = ids.len(), "refreshed MCP ToolRegistry");
                        }
                    }
                }
            });
        }

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
            .with_memory_hybrid(grok_hardening.memory_hybrid)
            .with_consolidation_repository(consolidation_repository)
            .with_retention_repository(retention_repository.clone()),
        );
        let gbrain_runtime = crate::adapters::gbrain::build_supplemental_memory_runtime_with_retention(
            local_memory,
            retained_mcp.clone(),
            &config.supplemental_memory,
            clock.clone(),
            &cancel_token,
            Some(retention_repository.clone()),
        );
        let memory_admin_use_cases: Arc<
            dyn crate::application::request_use_cases::MemoryAdminUseCases,
        > = Arc::new(
            crate::application::request_use_cases::ProductionMemoryAdminUseCases::new(
                gbrain_runtime.memory_service.clone(),
                retention_admin_port(retention_repository),
                fabric::LOCAL_OWNER_PRINCIPAL.to_string(),
            ),
        );
        let supplemental_memory_worker_task = gbrain_runtime.worker_task;
        let consolidation_cancel = cancel_token.clone();
        let consolidation_memory = gbrain_runtime.memory_service.clone();
        tokio::spawn(async move {
            crate::application::memory_consolidation_worker::MemoryConsolidationWorker::new(
                consolidation_memory,
            )
            .run(consolidation_cancel)
            .await;
        });

        let durable_budget = Arc::new(
            kernel::admission::DurableBudgetController::open_durable(
                data_dir.join("budget-controller-v1.json"),
            )
            .context("opening durable budget controller")?,
        );
        let kernel = Arc::new(kernel::KernelRuntime::with_clock_and_budget(
            clock.clone(),
            durable_budget,
        ));
        let hardware_clock: Arc<dyn hardware::MonotonicClock> =
            Arc::new(super::embodiment::HardwareClockAdapter(clock.clone()));
        let embodiment_workspace =
            fabric::WorkspacePolicy::from_resolved_roots(data_dir.clone(), Vec::new())
                .map_err(anyhow::Error::msg)
                .context("resolving embodiment workspace")?;
        let embodiment_port = super::embodiment::build_embodiment_port(
            hardware_clock,
            kernel.admission(),
            Arc::new(crate::application::embodiment_progress::NoopEmbodimentProgress),
            fabric::ProcessId::new(),
            fabric::PrincipalId(fabric::LOCAL_OWNER_PRINCIPAL.to_string()),
            embodiment_workspace,
            Some(config.embodiment_provider.clone()),
        )
        .await?;
        tools
            .lock()
            .await
            .register_robot_tools(embodiment_port)
            .context("registering governed robot tools")?;
        let fact_use_cases: Arc<dyn mnemosyne::FactUseCases> =
            Arc::new(mnemosyne::DefaultFactUseCases::new(fact_store.clone()));
        let goal_use_cases: Arc<dyn crate::application::GoalUseCases> = Arc::new(
            crate::application::GoalService::new(objective_store.clone()),
        );
        let runtime = Arc::new(Mutex::new(runtime));
        let admin_runtime = runtime.clone();
        let admin_tools = tools.clone();
        let skill_loader = Arc::new(Mutex::new(skill_loader));
        let admin_skill_loader = skill_loader.clone();
        let admin_hooks = hook_registry.clone();
        let cached_prefix = Arc::new(Mutex::new(cached_prefix));
        let admin_cached_prefix = cached_prefix.clone();
        let pending_approvals = crate::application::admin_service::PendingApprovals::default();
        let admin_pending_approvals = pending_approvals.clone();
        let session_approvals = crate::application::admin_service::ScopedApprovalCache::default();
        let admin_session_approvals = session_approvals.clone();
        let memory_queue = Arc::new(Mutex::new(Vec::new()));
        let dasein_handle = self_field
            .lock()
            .await
            .dasein_handle()
            .context("Dasein must be enabled for the recurrent conscious workspace")?;
        let agora_service: Arc<dyn fabric::AgoraService> =
            Arc::new(agora::AgoraRegistry::new(kernel.clock()));
        let conscious_registry = Arc::new(
            crate::application::conscious_workspace::ConsciousWorkspaceRegistry::production_with_mode_tools_and_agora(
                data_dir.join("conscious_workspace.db"),
                Arc::new(
                    crate::application::dasein_workspace_adapter::DaseinWorkspaceAdapter::new(
                        dasein_handle,
                        clock.clone(),
                    ),
                ),
                kernel.clone(),
                clock.clone(),
                gbrain_runtime.memory_service.clone(),
                skill_loader.clone(),
                tools.clone(),
                agora_service.clone(),
                config.conscious_arbitration_mode,
            )?,
        );
        conscious_context.bind(conscious_registry.clone())?;
        let context_source = Arc::new(
            crate::application::context_assembler::ProductionContextSource {
                cached_prefix: cached_prefix.clone(),
                skill_loader: skill_loader.clone(),
                skill_router: skill_router.clone(),
                conscious: conscious_context.clone(),
            },
        );
        let context_assembler =
            Arc::new(crate::application::context_assembler::ContextAssembler::new(context_source));
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
            agora_service,
            metacog,
            corpus.clone(),
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
            crate::host::daemon::handler::tool_executor::CapabilityResources {
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
            crate::host::daemon::handler::tool_executor::ProductionCapabilityService::new(
                capability_resources.clone(),
            ),
        );
        let agent_runtimes =
            Arc::new(crate::application::agent_control::AgentRuntimeRegistry::default());
        // Ordinary child Agents use one Cognit session runtime. Goal worker
        // and reviewer attempts remain explicit ProviderWorkerRuntime routes.
        let agent_composition = {
            // Stable agent control definitions must be visible to
            // load_agent_profiles so profiles can list them in `allowed_tools`
            // before the AgentControlService runtime is constructed.
            let mut definitions = corpus_group.tools.lock().await.definitions();
            definitions
                .extend(corpus::tools::tools::agent_control::AgentControlTools::definitions());
            let composition = super::agents::compose(super::agents::AgentCompositionInput {
                agents_dir: &aletheon_dir.join("agents"),
                inference: inference.clone(),
                default_llm: llm.clone(),
                definitions: &definitions,
                runtime_config: &runtime_config_snapshot,
                profiles_config: &agent_profiles,
            })?;
            let native = Arc::new(crate::adapters::runtime::NativeCognitRuntime::new(
                crate::adapters::runtime::NativeCognitRuntimeResources {
                    sessions: domains.cognition(),
                    capabilities: capability_service.clone(),
                    profiles: composition.profiles.clone(),
                    clock: clock.clone(),
                    conscious_actions: Some(conscious_registry.clone()),
                    conscious_candidates: Some(conscious_registry.clone()),
                },
            ));
            agent_runtimes.register(
                crate::adapters::runtime::NativeCognitRuntime::runtime_id(),
                native,
            )?;
            composition
        };
        let agent_profiles_for_tools = agent_composition.tool_profiles;
        let agent_profile_registry = agent_composition.profiles;
        let active_profile = Arc::new(Mutex::new(agent_composition.active_profile_name));

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
                        crate::application::agent_control::CompatibilityRuntimeLauncher::new(
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
                let runtime_id = crate::adapters::runtime::PI_CODER_RUNTIME_ID;
                let compatibility = runtime
                    .compatibility_runtimes()
                    .resolve(&fabric::RuntimeId(runtime_id.into()))?;
                agent_runtimes.register(
                    fabric::RuntimeId(runtime_id.into()),
                    Arc::new(
                        crate::application::agent_control::CompatibilityRuntimeLauncher::new(
                            compatibility,
                        ),
                    ),
                )?;
                info!(runtime_id = "pi-coder", "Pi coding runtime registered");
            }
            if let Some(pi_rpc) = pi_rpc {
                agent_runtimes.register_manifested(
                    PiRpcRuntime::runtime_id(),
                    Arc::new(pi_rpc),
                    crate::adapters::runtime::pi_manifest().clone(),
                )?;
                info!(runtime_id = "pi-rpc", "Pi resident RPC runtime registered");
            }
        }

        let goal_runtime_registry = if goal_runtime.enabled {
            let runtime = runtime.lock().await;
            Some(Arc::new(runtime.compatibility_runtimes().clone()))
        } else {
            None
        };

        let clock_2 = clock.clone();
        let agent_svc = super::services::build_agent_services(
            &data_dir,
            kernel.clone(),
            clock.clone(),
            cancel_token.clone(),
            config,
            &grok_hardening,
            domains.corpus(),
            agent_runtimes,
            corpus_group.tools.clone(),
            agent_profiles_for_tools,
            granted_capabilities.clone(),
            memory_group.memory_service.clone(),
        )
        .await?;
        let canonical_event_spine = agent_svc.canonical_event_spine;
        let agent_recovery = agent_svc.agent_recovery;
        let agent_repository = agent_svc.agent_repository;
        let turn_svc = super::services::build_turn_services(
            &data_dir,
            kernel.clone(),
            clock.clone(),
            cancel_token.clone(),
            event_bus.clone(),
            config,
            grok_hardening.clone(),
            &pi_runtime,
            pi_work_allowed,
            sessions.clone(),
            &session_id,
            initial_session.clone(),
            gw_state.clone(),
            gw_started_at,
            runtime_config_snapshot,
            core_memory.clone(),
            recall_memory.clone(),
            self_field.clone(),
            llm.clone(),
            debug_handler.clone(),
            debug_perf.clone(),
            model_router.clone(),
            &domains,
            &security_group,
            &memory_group,
            &session_group,
            capability_resources,
            conscious_registry.clone(),
            context_assembler,
            apply_objective_store,
            param_registry.clone(),
            agent_svc.agent_live_runs,
            canonical_event_spine.clone(),
            agent_svc.event_projections,
            agent_profile_registry.clone(),
            active_profile.clone(),
            runtime.clone(),
            turn_token.clone(),
            main_agent_process_id.clone(),
        )
        .await?;
        let session_input = turn_svc.session_input;
        let session_gateway = turn_svc.session_gateway;
        let turn_orchestrator = turn_svc.turn_orchestrator;
        let approved_apply = turn_svc.approved_apply;
        let lifecycle_registry = turn_svc.lifecycle_registry;

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
            let worker = crate::application::goal::GoalWorker::new(
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
        let external_sync = external_sync.map(|handle| {
            Arc::new(crate::adapters::google::GoogleSyncWorkerPort::new(handle))
                as Arc<dyn crate::application::admin_service::BackgroundWorkerPort>
        });
        let supplemental_memory_worker_task = supplemental_memory_worker_task.map(|task| Arc::new(Mutex::new(Some(task))));
        let self_field_shutdown = Arc::new(Mutex::new(Some(self_field.clone())));

        let approval_use_cases: Arc<dyn crate::application::ApprovalUseCases> =
            Arc::new(crate::application::ApprovalService::new(
                memory_group.approval_repository.clone(),
                approved_apply.clone(),
                clock.clone(),
                main_agent_process_id.clone(),
            ));
        let admin_use_cases: Arc<dyn crate::application::AdminUseCases> =
            Arc::new(crate::application::AdminService::new(
                crate::application::admin_service::AdminResources {
                    runtime: admin_runtime_port(admin_runtime),
                    skills: Arc::new(crate::composition::skill_admin::DefaultSkillAdmin::new(
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
                                .map(|hook| crate::application::admin_service::HookDescriptor {
                                    name: hook.name.clone(),
                                    source: hook.source.clone(),
                                    point: format!("{:?}", hook.point),
                                    priority: hook.priority,
                                    script_path: hook.script_path.clone(),
                                })
                                .collect()
                        })
                    }),
                    pending_approvals: admin_pending_approvals.clone(),
                    session_approvals: admin_session_approvals,
                    daemon_cancel: cancel_token.clone(),
                    external_sync: external_sync.clone(),
                    supplemental_memory_worker: supplemental_memory_worker_task.clone(),
                    goal_worker: goal_worker_task.clone(),
                    runtime_shutdown: Arc::new(move || {
                        let self_field_shutdown = self_field_shutdown.clone();
                        Box::pin(async move {
                            let mut pending = self_field_shutdown.lock().await;
                            let Some(self_field) = pending.as_ref() else {
                                return Ok(());
                            };
                            self_field.lock().await.shutdown().await.map_err(|error| {
                                crate::application::admin_service::AdminServiceError::Operation(
                                    error.to_string(),
                                )
                            })?;
                            pending.take();
                            Ok(())
                        })
                    }),
                    memory_admin: Some(memory_admin_use_cases),
                    agent_runs: Some(agent_repository),
                    agent_profiles: Some(agent_profile_registry),
                    current_profile: Some(active_profile),
                    profile_switch_events: Arc::new(
                        crate::application::admin_service::SpineProfileSwitchEventSink::new(
                            canonical_event_spine.clone(),
                        ),
                    ),
                    deployment_rollback: Some(Arc::new(
                        crate::core::deploy::DeploymentRollbackService::filesystem(
                            std::env::var_os("ALETHEON_DEPLOYMENT_MANIFEST")
                                .map(std::path::PathBuf::from)
                                .unwrap_or_else(|| data_dir.join("deployment-manifest.json")),
                        ),
                    )),
                },
            ));
        let legacy_sessions: Arc<
            dyn crate::application::legacy_session_service::LegacySessionUseCases,
        > = Arc::new(
            crate::application::legacy_session_service::LegacySessionService::new(
                crate::application::legacy_session_service::LegacySessionResources {
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
        let health_registry = Arc::new(crate::application::health::HealthRegistry::production_ready());
        let channel_task = Arc::new(Mutex::new(None));
        let request_facades = RequestFacadePorts::new(
            runtime.clone(),
            memory_group.episodic_memory.clone(),
            self_field.clone(),
            memory_group.supplemental_memory_health.clone(),
            grok_hardening.clone(),
        );
        let session_lifecycle: Arc<
            dyn crate::application::request_use_cases::SessionLifecycleUseCases,
        > = Arc::new(
            crate::application::request_use_cases::ProductionSessionLifecycle::new(
                domains.corpus(),
                security_group.session_approvals.clone(),
                turn_token.clone(),
                lifecycle_registry,
                grok_hardening.lifecycle_contributors,
            )
            .with_event_bus(event_bus.clone())
            .with_session_service(turn_orchestrator.session_service.clone()),
        );
        let health_use_cases: Arc<dyn crate::application::request_use_cases::HealthUseCases> =
            Arc::new(
                crate::application::request_use_cases::ProductionHealthUseCases::new(
                    crate::application::request_use_cases::ProductionHealthResources {
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
                        channel_task: channel_task.clone(),
                        external_sync: external_sync.clone(),
                        goal_worker: goal_worker_task.clone(),
                        agent_recovery: agent_recovery.clone(),
                    },
                ),
            );
        let reflection_use_cases: Arc<
            dyn crate::application::request_use_cases::ReflectionUseCases,
        > = Arc::new(
            crate::application::request_use_cases::ProductionReflectionUseCases::new(
                request_facades.runtime_port.clone(),
                request_facades.reflections,
                domains.metacog(),
                super::request_ports::reflection_engine_port(reflector.clone()),
            ),
        );
        let google_use_cases: Arc<dyn crate::application::request_use_cases::ExternalSourceUseCases> =
            Arc::new(
                crate::adapters::external::ProductionExternalSourceUseCases::new(
                    google.clone(),
                    domains.corpus(),
                    granted_capabilities.clone(),
                    clock.clone(),
                ),
            );
        let workflow_use_cases: Arc<dyn crate::application::request_use_cases::WorkflowUseCases> =
            Arc::new(
                crate::application::request_use_cases::ProductionWorkflowUseCases::new(
                    crate::application::orchestration::store::WorkflowStore::default_dir(),
                ),
            );
        let turn_use_cases: Arc<dyn crate::application::request_use_cases::TurnUseCases> = Arc::new(
            crate::application::request_use_cases::ProductionTurnUseCases::new(
                turn_orchestrator.clone(),
                request_facades.runtime_port,
                turn_token.clone(),
                turn_orchestrator.session_service.clone(),
            ),
        );
        let transport_ports = Arc::new(crate::host::daemon::handler::ports::TransportPorts {
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
        let handler_ports = Arc::new(crate::host::daemon::handler::ports::HandlerPorts::new(
            kernel.clone(),
            admin_pending_approvals.clone(),
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
            session_input,
            conscious_registry,
            debug_handler,
            session_gateway,
            transport_ports,
        ));
        let workspace_trust =
            bootstrap_workspace_trust_resolver(&data_dir, grok_hardening.folder_trust, event_bus);
        let composition = super::DaemonComposition {
            request: handler_ports,
            active_connections,
            thread_authority: Arc::new(
                crate::application::thread_authority::ThreadAuthorityStore::persistent(
                    data_dir.join("thread-authority"),
                ),
            ),
            grok_hardening,
            workspace_trust,
            mcp: retained_mcp,
        };
        let handler = composition.into_handler();

        super::params::register_initial_params(
            &param_registry,
            clock_2.clone(),
            data_dir.clone(),
            config.model.clone(),
            llm.clone(),
            config.sandbox_preference.clone(),
        )
        .await;

        // Fire OnSessionStart hook
        handler
            .ports
            .session_lifecycle
            .start(session_id.clone(), false)
            .await?;

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
            *channel_task.lock().await = Some(Arc::new(jh));
        } else {
            info!("Telegram channel disabled");
        }

        Ok(handler)
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
