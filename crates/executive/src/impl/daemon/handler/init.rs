//! Handler initialization and construction.
//!
//! Contains the `RequestHandler::new()` constructor and setup-related methods
//! (`set_notify_channel`, `create_notify_channel`, `tools`, `debug_handler`).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

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
use crate::core::sub_agent::SubAgentSpawner;
use crate::session::store::SessionStore;
use cognit::core::reflector::Reflector;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::{create_default_executor, SandboxPreference};
use corpus::security::socket_approval::SocketApprovalGate;
use corpus::tools::google::{
    CalendarSyncConfig, CalendarSynchronizer, DriveSyncConfig, DriveSynchronizer,
    GmailHistorySyncConfig, GmailHistorySynchronizer, GoogleApiClient, GoogleApiEndpoints,
    GoogleCalendarAdapter, GoogleDriveAdapter, GoogleGmailAdapter,
};
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
use mnemosyne::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};
use mnemosyne::CoreMemory;
use mnemosyne::RecallMemory;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::r#impl::agent_loader::AgentLoader;
use crate::r#impl::channel::daemon_adapter::{
    DaemonChannelApprovalExecutor, DaemonChannelGoalExecutor, DaemonChannelTurnExecutor,
    DaemonGmailDraftApprovalExecutor,
};
use crate::r#impl::channel::gmail::GmailGoalDraftCoordinator;
use crate::r#impl::channel::router::{
    ChannelApprovalExecutor, ChannelGoalExecutor, ChannelRouter, ChannelTransport,
    ChannelTurnExecutor, GmailDraftApprovalExecutor,
};
use crate::r#impl::channel::store::ChannelStore;
use crate::r#impl::channel::telegram::TelegramTransport;
use crate::r#impl::external::{
    ExecutiveGoogleAccountResolver, ExecutiveGoogleCredentialSource, ExternalIdentityRepository,
    GoogleIntegration,
};
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::runtime::worktree_recovery::{WorktreeRecoveryConfig, WorktreeRecoveryService};
use crate::r#impl::runtime::{register_pi_runtime, ProviderWorkerRuntime};
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
use fabric::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

pub(crate) fn register_goal_runtimes(
    spawner: &mut SubAgentSpawner,
    config: &cognit::config::GoalRuntimeConfig,
    providers: &ProviderRegistry,
    tools: Arc<Mutex<ToolRegistry>>,
    clock: Arc<dyn Clock>,
) -> anyhow::Result<Vec<fabric::RuntimeId>> {
    if !config.enabled {
        return Ok(Vec::new());
    }
    let worker = config
        .worker
        .as_ref()
        .context("goal runtime is enabled but worker routing is missing")?;
    let reviewer = config
        .reviewer
        .as_ref()
        .context("goal runtime is enabled but reviewer routing is missing")?;
    if worker.runtime_id == reviewer.runtime_id {
        anyhow::bail!("worker and reviewer runtime IDs must be distinct");
    }

    let routes = [
        (worker, fabric::CognitiveRole::Worker),
        (reviewer, fabric::CognitiveRole::Reviewer),
    ];
    let mut prepared = Vec::with_capacity(routes.len());
    for (route, role) in routes {
        if route.runtime_id.trim().is_empty() {
            anyhow::bail!("goal runtime ID must not be empty");
        }
        let (provider_config, model) =
            providers
                .resolve_role_alias(&route.model_alias)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "resolving runtime '{}': {}: {error}",
                        route.runtime_id,
                        route.model_alias
                    )
                })?;
        let provider: Arc<dyn LlmProvider> =
            Arc::from(providers.create_provider(&provider_config, &model));
        let runtime_id = fabric::RuntimeId(route.runtime_id.clone());
        let runtime = Arc::new(ProviderWorkerRuntime::new(
            runtime_id.clone(),
            role,
            provider,
            tools.clone(),
            clock.clone(),
            route.max_steps,
            route.max_persisted_bytes,
            route.allowed_tools.clone(),
        ));
        prepared.push((runtime_id, runtime));
    }

    let mut registered = Vec::with_capacity(prepared.len());
    for (runtime_id, runtime) in prepared {
        spawner
            .runtime_registry_mut()
            .register(runtime_id.clone(), runtime)?;
        registered.push(runtime_id);
    }
    Ok(registered)
}

fn register_configured_google_read_tools(
    tools: &mut ToolRegistry,
    objective_db_path: &std::path::Path,
    clock: Arc<dyn Clock>,
    cancel: &CancellationToken,
) -> anyhow::Result<
    Option<(
        Arc<GoogleIntegration>,
        crate::r#impl::google::GoogleSyncHandle,
    )>,
> {
    let client_id = match std::env::var("ALETHEON_GOOGLE_CLIENT_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let redirect_uri = std::env::var("ALETHEON_GOOGLE_REDIRECT_URI")
        .context("ALETHEON_GOOGLE_REDIRECT_URI is required when Google is configured")?;
    let client_secret = std::env::var("ALETHEON_GOOGLE_CLIENT_SECRET")
        .ok()
        .filter(|value| !value.is_empty());

    let repository = ExternalIdentityRepository::open(objective_db_path)
        .context("opening external identity repository")?;
    let active_bindings = repository.list_active()?;
    let gmail_enabled = repository.has_active_scope(fabric::ExternalScope::GmailReadonly)?;
    let calendar_enabled = repository.has_active_scope(fabric::ExternalScope::CalendarReadonly)?;
    let mut scopes = vec![
        fabric::ExternalScope::OpenId,
        fabric::ExternalScope::UserInfoEmail,
        fabric::ExternalScope::GmailReadonly,
        fabric::ExternalScope::CalendarReadonly,
    ];
    if std::env::var("ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
    {
        scopes.push(fabric::ExternalScope::DriveReadonly);
    }
    let tokens = corpus::tools::mcp::token_store::TokenStore::open_default()
        .context("opening encrypted Google credential vault")?;
    let oauth = corpus::tools::google::oauth::GoogleOAuthProvider::new(
        client_id,
        client_secret,
        redirect_uri,
        scopes,
        tokens,
        clock.clone(),
    )?;
    let repository = Arc::new(std::sync::Mutex::new(repository));
    let oauth = Arc::new(Mutex::new(oauth));
    let integration = Arc::new(GoogleIntegration::new(repository.clone(), oauth.clone()));
    let credentials = Arc::new(ExecutiveGoogleCredentialSource::new(
        repository.clone(),
        oauth,
    ));
    let accounts = Arc::new(ExecutiveGoogleAccountResolver::new(repository));
    let client = GoogleApiClient::new(credentials, GoogleApiEndpoints::default())?;
    let gmail = gmail_enabled.then(|| {
        Arc::new(GoogleGmailAdapter::new(client.clone()))
            as Arc<dyn corpus::tools::google::GmailCapability>
    });
    let calendar = calendar_enabled.then(|| {
        Arc::new(GoogleCalendarAdapter::new(client.clone()))
            as Arc<dyn corpus::tools::google::CalendarCapability>
    });
    tools.register_google_read_tools(gmail, calendar, accounts)?;

    let store = Arc::new(std::sync::Mutex::new(
        crate::r#impl::google::GoogleSyncStore::open(objective_db_path)?,
    ));
    let mut manager = crate::r#impl::google::GoogleSyncManager::new(
        store,
        format!("daemon-{}", uuid::Uuid::new_v4()),
        clock.clone(),
        crate::r#impl::google::GoogleSyncManagerConfig::default(),
    )?;
    let drive_enabled = std::env::var("ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"));
    let selected_drive_files = std::env::var("ALETHEON_GOOGLE_DRIVE_FILE_IDS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    let now_ms = clock.wall_now().0.max(0);
    for (identity, grant) in active_bindings {
        if grant.scopes.contains(&fabric::ExternalScope::GmailReadonly) {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::GmailHistory,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::GmailHistoryPoller(
                    GmailHistorySynchronizer::new(
                        GoogleGmailAdapter::new(client.clone()),
                        GmailHistorySyncConfig::default(),
                    )?,
                )),
            })?;
        }
        if grant
            .scopes
            .contains(&fabric::ExternalScope::CalendarReadonly)
        {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::Calendar,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::CalendarDeltaPoller(
                    CalendarSynchronizer::new(
                        GoogleCalendarAdapter::new(client.clone()),
                        CalendarSyncConfig {
                            window_start_ms: now_ms.saturating_sub(30 * 86_400_000),
                            window_end_ms: now_ms.saturating_add(365 * 86_400_000),
                            timezone: "UTC".into(),
                            max_pages: 20,
                            page_size: 250,
                        },
                    )?,
                )),
            })?;
        }
        if drive_enabled && grant.scopes.contains(&fabric::ExternalScope::DriveReadonly) {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id,
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::DriveChanges,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::DriveChangesPoller(
                    DriveSynchronizer::new(
                        GoogleDriveAdapter::new(client.clone()),
                        DriveSyncConfig {
                            selected_file_ids: selected_drive_files.clone(),
                            content_mime_allowlist: std::collections::HashSet::new(),
                            download_content: false,
                            max_content_bytes: 8 * 1_048_576,
                            max_pages: 20,
                            max_changes: 2_000,
                            page_size: 100,
                        },
                    )?,
                )),
            })?;
        }
    }
    let sync = manager.start(cancel);
    Ok(Some((integration, sync)))
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
        goal_runtime: cognit::config::GoalRuntimeConfig,
        pi_runtime: cognit::config::PiRuntimeConfig,
        evolution_enabled: bool,
        event_bus: Option<Arc<CommunicationBus>>,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create("")?);
        info!(provider = llm.name(), "LLM provider initialized");

        // Create session and journal
        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let data_dir_for_telegram = data_dir.clone();
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
        let objective_db_path = aletheon_dir.join("objectives.db");
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
        let clock = Arc::new(SystemClock::new());

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
        let (google, google_sync) = match register_configured_google_read_tools(
            &mut tools,
            &objective_db_path,
            clock.clone(),
            &cancel_token,
        ) {
            Ok(Some((integration, sync))) => (Some(integration), Some(sync)),
            Ok(None) => (None, None),
            Err(error) => {
                warn!(error = %error, "Google read integration disabled");
                (None, None)
            }
        };

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
                let required = if config.gbrain_memory.capture_enabled {
                    &["query", "get_page", "put_page"][..]
                } else {
                    &["query", "get_page"][..]
                };
                if mcp.server_has_tools(&config.gbrain_memory.server_name, required) {
                    retained_mcp = Some(Arc::new(mcp));
                } else {
                    tracing::warn!(
                        server = %config.gbrain_memory.server_name,
                        ?required,
                        "gbrain disabled: server disconnected or required tools missing"
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
        let gw_started_at = clock.mono_now();
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
            clock.clone(),
        ));

        let ports = aletheon_kernel::service::ServicePorts::new()
            .with_agora(Arc::new(agora::AgoraRegistry::new()));
        let subsystems = Arc::new(crate::core::core_systems::CoreSystems {
            ports,
            runtime: Arc::new(Mutex::new(runtime)),
            self_field,
            reflector,
            memory: crate::core::MemoryGroup {
                gbrain: retained_mcp,
                gbrain_config: config.gbrain_memory.clone(),
                memory_service: Arc::new(mnemosyne::DefaultMemoryService::new(
                    recall_memory.clone(),
                    fact_store.clone(),
                    core_memory.clone(),
                    episodic_memory.clone(),
                    clock.clone(),
                )),
                episodic_memory,
                recall_memory,
                core_memory,
                fact_store,
                auto_memory,
                objective_store,
                approval_repository,
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
            let allowed_tools = subsystems
                .corpus
                .tools
                .lock()
                .await
                .list()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let sub_agent_runtime = Arc::new(ProviderWorkerRuntime::new(
                fabric::RuntimeId("default".into()),
                fabric::CognitiveRole::Worker,
                llm.clone(),
                subsystems.corpus.tools.clone(),
                clock.clone(),
                8,
                16 * 1024,
                allowed_tools,
            ));
            subsystems
                .runtime
                .lock()
                .await
                .sub_agent_spawner_mut()
                .with_runtime(sub_agent_runtime);
        }

        // Goal worker/reviewer runtimes are opt-in and strictly alias-resolved.
        // Missing routes fail startup only when Goal execution is enabled.
        {
            let mut runtime = subsystems.runtime.lock().await;
            let registered = register_goal_runtimes(
                runtime.sub_agent_spawner_mut(),
                &goal_runtime,
                registry,
                subsystems.corpus.tools.clone(),
                clock.clone(),
            )?;
            if !registered.is_empty() {
                info!(runtime_ids = ?registered, "Goal runtimes registered");
            }
        }

        // Coding jobs are fail-closed: only a probed namespace backend may
        // register the stable `pi-coder` runtime ID.
        {
            let sandbox = corpus::security::sandbox::BubblewrapBackend::probe_async(clock.clone())
                .await
                .map(|backend| Arc::new(backend) as Arc<dyn fabric::sandbox::SandboxBackend>);
            let mut runtime = subsystems.runtime.lock().await;
            if pi_work_allowed
                && register_pi_runtime(
                    runtime.sub_agent_spawner_mut(),
                    &pi_runtime,
                    sandbox,
                    clock.clone(),
                )?
            {
                info!(runtime_id = "pi-coder", "Pi coding runtime registered");
            }
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

        // Clone clock before the agent-tool closure consumes the original binding.
        let clock_2 = clock.clone();

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

                let clock_for_agents = clock.clone();
                let execute_fn: corpus::tools::tools::agent_tool::ExecuteSubAgentFn =
                    Arc::new(move |system_prompt, user_prompt, allowed_tools| {
                        let llm = llm_for_agents.clone();
                        let tools = tools_for_agents.clone();
                        let exec = exec_for_agents.clone();
                        let main_slot = main_slot.clone();
                        let sp = system_prompt;
                        let up = user_prompt;
                        let at = allowed_tools;
                        let clock = clock_for_agents.clone();
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
            clock_2.mono_now(),
            Some(cancel_token.clone()),
        ));

        let approved_apply = if pi_runtime.enabled && pi_work_allowed {
            Some(Arc::new(crate::r#impl::approval::ApplyCoordinator::new(
                apply_objective_store,
                subsystems.memory.approval_repository.clone(),
                subsystems.ports.operation_table.clone(),
                clock.clone(),
                crate::r#impl::approval::ApplyCoordinatorConfig {
                    worktree_base: pi_runtime.worktree_base.clone(),
                    timeout: std::time::Duration::from_secs(60),
                },
                Arc::new(crate::r#impl::approval::GitManagedWorktreeCleaner),
            )?))
        } else {
            None
        };

        // Clone these before they are moved into the handler struct
        // so they are available for Telegram channel initialization.
        let _turn_orch_for_telegram = turn_orchestrator.clone();
        let _cancel_for_telegram = cancel_token.clone();

        let mut handler = Self {
            subsystems,
            sessions,
            session_gateway,
            bus,
            llm,
            model_router,
            notify_tx: None,
            active_connections,
            started_at: clock_2.mono_now(),
            daemon_cancel_token: Some(cancel_token),
            turn_orchestrator,
            telegram_task: None,
            google_sync: google_sync.map(|handle| Arc::new(Mutex::new(Some(handle)))),
            approved_apply: approved_apply.clone(),
            google,
        };

        // Register initial params
        {
            let data_dir_clone = handler.subsystems.session.data_dir.clone();
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

        // -- Telegram channel initialization (M1) -------------------------------
        let telegram_cfg = &config.telegram;
        if telegram_cfg.enabled {
            info!("Telegram channel enabled — initializing owner-only control channel");
            let jh = Self::init_telegram_channel(
                telegram_cfg,
                data_dir_for_telegram,
                _turn_orch_for_telegram,
                handler.subsystems.memory.objective_store.clone(),
                handler.subsystems.memory.approval_repository.clone(),
                gmail_goal_drafts,
                approved_apply,
                handler.google.clone(),
                _cancel_for_telegram,
            );
            handler.telegram_task = Some(Arc::new(jh));
        } else {
            info!("Telegram channel disabled");
        }

        Ok(handler)
    }

    /// Build the Telegram long-poll channel transport, router, and spawn the
    /// poll loop. Returns the task handle for graceful shutdown.
    fn init_telegram_channel(
        cfg: &cognit::config::TelegramConfig,
        data_dir: PathBuf,
        orchestrator: Arc<crate::service::DaemonTurnOrchestrator>,
        objective_store: Arc<Mutex<ObjectiveStore>>,
        approval_repository: Arc<std::sync::Mutex<crate::r#impl::approval::ApprovalRepository>>,
        gmail_goal_drafts: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>,
        approved_apply: Option<Arc<crate::r#impl::approval::ApplyCoordinator>>,
        google: Option<Arc<GoogleIntegration>>,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let store_path = data_dir.join("channels.db");
        let store = ChannelStore::open(&store_path).expect("opening channel store for Telegram");
        let cursor: Option<String> = store.cursor("telegram").unwrap_or(None);

        if let Some(owner_id) = cfg.owner_user_id {
            let external = format!("telegram:{}", owner_id);
            store
                .bind("telegram", &external, "owner", "active")
                .expect("binding Telegram owner");
            info!(owner_id = owner_id, "Telegram owner binding seeded");
        } else {
            warn!("Telegram enabled but owner_user_id not set");
        }

        let token = cfg
            .bot_token_env
            .as_ref()
            .and_then(|env_name| std::env::var(env_name).ok())
            .unwrap_or_default();
        if token.is_empty() {
            warn!(
                env = ?cfg.bot_token_env,
                "Telegram bot token not found in environment"
            );
        }

        let poll_timeout = cfg.poll_timeout_secs.clamp(1, 50);
        let transport = TelegramTransport::new(token, None, poll_timeout, cancel.clone());

        let turn_executor: Arc<dyn ChannelTurnExecutor> =
            Arc::new(DaemonChannelTurnExecutor::new(orchestrator));

        let goal_executor: Arc<dyn ChannelGoalExecutor> =
            Arc::new(DaemonChannelGoalExecutor::new(objective_store));
        let approval_repository_for_poll = approval_repository.clone();
        let approval_conversation = cfg
            .owner_user_id
            .map(|id| fabric::channel::ConversationId(id.to_string()));
        let mut router = ChannelRouter::new(store, turn_executor)
            .with_goal_executor(goal_executor)
            .with_approval_repository(approval_repository);
        let gmail_executor: Arc<dyn GmailDraftApprovalExecutor> =
            Arc::new(DaemonGmailDraftApprovalExecutor::new(gmail_goal_drafts));
        router = router.with_gmail_draft_executor(gmail_executor);
        if let Some(google) = google {
            router = router.with_google_accounts(google);
        }
        if let Some(coordinator) = approved_apply {
            let executor: Arc<dyn ChannelApprovalExecutor> =
                Arc::new(DaemonChannelApprovalExecutor::new(
                    coordinator,
                    fabric::ProcessId::new(),
                    cancel.clone(),
                ));
            router = router.with_approval_executor(executor);
        }

        tokio::spawn(async move {
            Self::telegram_poll_loop(
                router,
                transport,
                cursor,
                approval_repository_for_poll,
                approval_conversation,
                cancel,
            )
            .await;
        })
    }

    /// Long-poll loop with jittered exponential backoff and cancellation.
    async fn telegram_poll_loop(
        mut router: ChannelRouter,
        transport: TelegramTransport,
        mut cursor: Option<String>,
        approval_repository: Arc<std::sync::Mutex<crate::r#impl::approval::ApprovalRepository>>,
        approval_conversation: Option<fabric::channel::ConversationId>,
        cancel: CancellationToken,
    ) {
        let mut backoff_ms: u64 = 1_000;
        let max_backoff_ms: u64 = 60_000;

        loop {
            if cancel.is_cancelled() {
                info!("Telegram poll loop exited (cancel token fired)");
                break;
            }

            if let Some(conversation) = &approval_conversation {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let pending = approval_repository
                    .lock()
                    .unwrap()
                    .list_pending(&fabric::PrincipalId("owner".into()), now_ms);
                match pending {
                    Ok(pending) => {
                        for approval in pending {
                            if let Err(error) = router
                                .notify_approval(
                                    &transport,
                                    conversation.clone(),
                                    &approval,
                                    now_ms,
                                )
                                .await
                            {
                                warn!(approval_id = %approval.id, error = %error, "Telegram approval notification failed");
                            }
                        }
                    }
                    Err(error) => {
                        warn!(error = %error, "Loading pending Telegram approvals failed")
                    }
                }
            }

            let result = tokio::select! {
                _ = cancel.cancelled() => {
                    info!("Telegram poll loop cancelled during receive wait");
                    break;
                }
                r = transport.receive(cursor.clone()) => r,
            };

            match result {
                Ok(envelopes) => {
                    backoff_ms = 1_000;
                    if envelopes.is_empty() {
                        continue;
                    }
                    let mut sorted: Vec<_> = envelopes;
                    sorted.sort_by_key(|e| e.message.message_id.0.parse::<i64>().unwrap_or(0));
                    for envelope in sorted {
                        let next_cursor = envelope.next_cursor.clone();
                        match router.process(&transport, envelope).await {
                            Ok(()) => {
                                cursor = Some(next_cursor);
                            }
                            Err(e) => {
                                warn!(error = %e, "Telegram router process failed");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e.to_string(), backoff_ms, "Telegram receive error, backing off");
                    if cancel.is_cancelled() {
                        break;
                    }
                    let jitter_ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos())
                        .unwrap_or(0);
                    let jitter_ms = (backoff_ms / 4).saturating_mul(jitter_ns as u64 % 101 / 100);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms + jitter_ms))
                        .await;
                    backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
                }
            }
        }
    }
}

#[cfg(test)]
mod goal_runtime_tests {
    use super::*;
    use cognit::config::{
        AppConfig, GoalRuntimeConfig, ProviderConfig, RoleRuntimeConfig, Transport,
    };

    fn provider(name: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: String::new(),
            transport: Transport::Openai,
            models: vec!["model".into()],
            max_context_length: None,
            pricing: None,
        }
    }

    fn route(runtime_id: &str, model_alias: &str) -> RoleRuntimeConfig {
        RoleRuntimeConfig {
            runtime_id: runtime_id.into(),
            model_alias: model_alias.into(),
            max_steps: 2,
            max_persisted_bytes: 1024,
            allowed_tools: vec![],
        }
    }

    fn register(
        config: GoalRuntimeConfig,
        app: AppConfig,
    ) -> anyhow::Result<(SubAgentSpawner, Vec<fabric::RuntimeId>)> {
        let providers = ProviderRegistry::from_config(&app)?;
        let mut spawner = SubAgentSpawner::new();
        let ids = register_goal_runtimes(
            &mut spawner,
            &config,
            &providers,
            Arc::new(Mutex::new(ToolRegistry::new())),
            Arc::new(SystemClock::new()),
        )?;
        Ok((spawner, ids))
    }

    #[test]
    fn disabled_goal_runtime_registers_nothing() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let (spawner, ids) = register(GoalRuntimeConfig::default(), app).unwrap();
        assert!(ids.is_empty());
        assert!(!spawner
            .runtime_registry()
            .contains(&fabric::RuntimeId("deepseek-worker".into())));
    }

    #[test]
    fn enabled_goal_runtime_rejects_missing_route_and_unknown_alias() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let missing = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "p/model")),
            reviewer: None,
        };
        assert!(register(missing, app.clone())
            .unwrap_err()
            .to_string()
            .contains("reviewer routing is missing"));

        let unknown = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "unknown-alias")),
            reviewer: Some(route("escalation-reviewer", "p/model")),
        };
        assert!(register(unknown, app)
            .unwrap_err()
            .to_string()
            .contains("model alias 'unknown-alias' not found"));
    }

    #[test]
    fn same_provider_can_back_distinct_runtime_ids() {
        let mut app = AppConfig::default();
        app.providers.push(provider("shared"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "shared/worker-model")),
            reviewer: Some(route("escalation-reviewer", "shared/reviewer-model")),
        };
        let (spawner, ids) = register(config, app).unwrap();
        assert_eq!(ids.len(), 2);
        for id in ids {
            assert!(spawner.runtime_registry().contains(&id));
        }
    }

    #[test]
    fn distinct_providers_register_worker_and_reviewer() {
        let mut app = AppConfig::default();
        app.providers.push(provider("worker-provider"));
        app.providers.push(provider("review-provider"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "worker-provider/model")),
            reviewer: Some(route("escalation-reviewer", "review-provider/model")),
        };
        let (spawner, ids) = register(config, app).unwrap();
        assert_eq!(
            ids,
            vec![
                fabric::RuntimeId("deepseek-worker".into()),
                fabric::RuntimeId("escalation-reviewer".into())
            ]
        );
        assert!(spawner.runtime_registry().contains(&ids[0]));
        assert!(spawner.runtime_registry().contains(&ids[1]));
    }
}
