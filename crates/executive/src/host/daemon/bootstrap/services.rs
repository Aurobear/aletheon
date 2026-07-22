//! Agent control service, session infrastructure, turn pipeline, and
//! orchestrator construction extracted from the handler bootstrap.
//!
//! Each builder function returns intermediate state consumed by the next
//! stage, mirroring the linear dependency order in `RequestHandler::new`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::request_ports::{post_turn_runtime_port, TurnRuntimeFacadePorts};
use crate::application::agent_control::SqliteAgentRunRepository;
use crate::composition::config::GrokHardeningConfig;
use crate::core::session_gateway::{ParamRegistry, SessionGateway};
use crate::core::DomainPorts;
use crate::core::MemoryGroup;
use crate::core::SecurityGroup;
use crate::core::SessionGroup;
use crate::host::daemon::session_manager::SessionManager;
use crate::host::daemon::DaemonConfig;

// ── Stage 1: agent control service ──────────────────────────────────────

pub(super) struct AgentServices {
    pub agent_recovery: crate::application::agent_control::AgentRecoveryReport,
    pub agent_repository: Arc<SqliteAgentRunRepository>,
    pub canonical_event_spine: Arc<crate::adapters::events::SqliteEventSpine>,
    pub event_projections: Arc<crate::adapters::events::DefaultEventProjectionSet>,
    pub agent_live_runs: Arc<crate::application::agent_control::LiveAgentRuns>,
}

pub(super) async fn build_agent_services(
    data_dir: &std::path::Path,
    kernel: Arc<kernel::KernelRuntime>,
    clock: Arc<dyn fabric::Clock>,
    cancel_token: CancellationToken,
    config: &DaemonConfig,
    grok_hardening: &GrokHardeningConfig,
    corpus: Arc<dyn corpus::CorpusService>,
    agent_runtimes: Arc<crate::application::agent_control::AgentRuntimeRegistry>,
    tools: Arc<Mutex<corpus::tools::tools::ToolRegistry>>,
    agent_profiles_for_tools: HashMap<String, fabric::AgentProfile>,
    granted_capabilities: Arc<tokio::sync::RwLock<Vec<fabric::CapabilityId>>>,
    durable_memory: Arc<dyn mnemosyne::MemoryService>,
) -> anyhow::Result<AgentServices> {
    let agent_state_root = data_dir.join("agents");
    std::fs::create_dir_all(&agent_state_root)?;
    let agent_repository = Arc::new(
        SqliteAgentRunRepository::open(agent_state_root.join("agent_control.db"))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let canonical_event_spine = Arc::new(
        crate::adapters::events::SqliteEventSpine::open(data_dir.join("events.db"))
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "canonical event spine unavailable; using process-local fallback");
                crate::adapters::events::SqliteEventSpine::open(":memory:")
                    .expect("in-memory event spine")
            }),
    );
    let event_projections = Arc::new(
        crate::adapters::events::DefaultEventProjectionSet::open(
            data_dir.join("event-projections.db"),
        )
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "event projections unavailable; using process-local fallback");
            crate::adapters::events::DefaultEventProjectionSet::in_memory()
        }),
    );
    let agent_daemon_generation = format!("daemon:{}", uuid::Uuid::new_v4());
    let settlement_receipts = Arc::new(
        crate::application::agent_control::SqliteSettlementReceiptStore::open(
            agent_state_root.join("agent_settlement.db"),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let agent_control_service = Arc::new(
        crate::application::agent_control::AgentControlService::new(
            kernel.clone(),
            clock.clone(),
            agent_repository.clone(),
            Arc::new(
                crate::application::agent_control::BoundedAgentAdmission::with_budget(
                    config.agent_admission.clone(),
                    kernel.budget_controller(),
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
            ),
            agent_runtimes,
        )
        .with_budget_controller(kernel.budget_controller())
        .with_event_spine(canonical_event_spine.clone())
        .with_event_projections(event_projections.clone())
        .with_lifecycle_hooks(Arc::new(
            crate::application::agent_control::CorpusAgentLifecycleHookSink(corpus),
        ))
        .with_memory_vault(Arc::new(
            mnemosyne::AgentMemoryVault::open(agent_state_root.join("agent_memory.db"))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        ))
        .with_durable_memory(durable_memory)
        .with_subagent_settlement(
            grok_hardening.subagent_settlement,
            agent_daemon_generation.clone(),
            settlement_receipts,
        ),
    );
    let agent_recovery = agent_control_service
        .reconcile_startup(&agent_daemon_generation)
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
    let agent_cleanup = crate::application::agent_control::AgentCleanupCoordinator::new(
        agent_repository.clone(),
        Arc::new(
            crate::adapters::runtime::worktree_recovery::VerifiedAgentWorktreeReclaimer::default(),
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
    let agent_live_runs = agent_control_service.live_runs();
    let agent_shutdown_cancel = cancel_token.clone();
    tokio::spawn(async move {
        agent_shutdown_cancel.cancelled().await;
        agent_control_service.shutdown().await;
    });

    super::runtime::register_agent_tools(tools.clone(), agent_control, agent_profiles_for_tools)
        .await;
    *granted_capabilities.write().await = corpus::discover_tool_extensions(&tools)
        .await?
        .into_iter()
        .flat_map(|entry| entry.capabilities)
        .collect();

    Ok(AgentServices {
        agent_recovery,
        agent_repository,
        canonical_event_spine,
        event_projections,
        agent_live_runs,
    })
}

// ── Stage 2: session infrastructure + turn pipeline ─────────────────────

pub(super) struct TurnServices {
    pub session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
    pub session_gateway: Arc<SessionGateway>,
    pub turn_orchestrator: Arc<crate::application::DaemonTurnOrchestrator>,
    pub approved_apply: Option<Arc<crate::application::approval::ApplyCoordinator>>,
    pub lifecycle_registry: Arc<crate::application::lifecycle_contributors::LifecycleRegistry>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_turn_services(
    data_dir: &std::path::Path,
    kernel: Arc<kernel::KernelRuntime>,
    clock: Arc<dyn fabric::Clock>,
    cancel_token: CancellationToken,
    event_bus: Option<Arc<fabric::CanonicalEventBus>>,
    config: &DaemonConfig,
    grok_hardening: GrokHardeningConfig,
    pi_runtime: &crate::composition::config::CodingRuntimeConfig,
    pi_work_allowed: bool,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    session_id: &str,
    initial_session: Arc<Mutex<SessionManager>>,
    gw_state: Arc<Mutex<crate::core::session_gateway::SessionStateRef>>,
    gw_started_at: fabric::MonoTime,
    runtime_config_snapshot: crate::composition::config::ExecutiveConfig,
    core_memory: Arc<Mutex<mnemosyne::runtime::CoreMemory>>,
    recall_memory: Arc<Mutex<mnemosyne::runtime::RecallMemory>>,
    self_field: Arc<Mutex<dasein::SelfField>>,
    llm: Arc<dyn fabric::LlmProvider>,
    debug_handler: Arc<crate::host::daemon::debug_handler::DebugHandler>,
    debug_perf: Arc<fabric::kernel::debug_bus::PerfCounter>,
    model_router: Arc<super::super::model_router::ModelRouter>,
    domains: &DomainPorts,
    security_group: &SecurityGroup,
    memory_group: &MemoryGroup,
    session_group: &SessionGroup,
    capability_resources: crate::host::daemon::handler::tool_executor::CapabilityResources,
    conscious_registry: Arc<crate::application::conscious_workspace::ConsciousWorkspaceRegistry>,
    context_assembler: Arc<crate::application::context_assembler::ContextAssembler>,
    apply_objective_store: Arc<std::sync::Mutex<crate::application::goal::ObjectiveStore>>,
    param_registry: Arc<ParamRegistry>,
    agent_live_runs: Arc<crate::application::agent_control::LiveAgentRuns>,
    canonical_event_spine: Arc<crate::adapters::events::SqliteEventSpine>,
    event_projections: Arc<crate::adapters::events::DefaultEventProjectionSet>,
    agent_profile_registry: Arc<crate::adapters::runtime::AgentProfileRegistry>,
    active_profile: Arc<Mutex<String>>,
    runtime: Arc<Mutex<crate::core::orchestrator::AletheonExecutive>>,
    turn_token: Arc<Mutex<Option<CancellationToken>>>,
    main_agent_process_id: Arc<Mutex<Option<fabric::ProcessId>>>,
) -> anyhow::Result<TurnServices> {
    let shared_notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>> = Arc::new(Mutex::new(None));
    let session_id = session_id.to_owned();
    let session_db = data_dir.join("sessions-v1.db");
    if let Some(parent) = session_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let canonical_store =
        crate::adapters::session::canonical_store::CanonicalSessionStore::open(&session_db)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, path = %session_db.display(), "canonical session store unavailable; using process-local fallback");
                crate::adapters::session::canonical_store::CanonicalSessionStore::open(":memory:")
                    .expect("in-memory canonical session store")
            });
    let session_recovery =
        crate::adapters::session::event_sourced_store::reconcile_committed_session_events(
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

    let turn_recovery_report =
        crate::application::turn_recovery::scan_incomplete_turns(&canonical_store, &grok_hardening)
            .await
            .context("incomplete-turn recovery scan during daemon startup")?;
    crate::application::turn_recovery::persist_recovery_health(data_dir, &turn_recovery_report)
        .context("persist turn recovery health")?;
    if !turn_recovery_report.incomplete_turns.is_empty() {
        for turn in &turn_recovery_report.incomplete_turns {
            info!(
                session = %turn.session_id,
                turn = %turn.turn_id,
                classification = ?turn.classification,
                items = turn.item_count,
                "Recovered incomplete turn at startup"
            );
        }
    }

    let session_input = if grok_hardening.prompt_queue {
        let coordinator =
            crate::application::session_input::SessionInputCoordinator::new(Arc::new(
                crate::adapters::session::prompt_queue_sqlite::SqlitePromptQueueStore::open(
                    data_dir.join("prompt-queue.sqlite"),
                )?,
            ))
            .with_event_spine(canonical_event_spine.clone());
        Arc::new(if let Some(bus) = event_bus.as_ref() {
            coordinator.with_event_bus(bus.clone())
        } else {
            coordinator
        })
    } else {
        Arc::new(crate::application::session_input::SessionInputCoordinator::in_memory())
    };
    let coordinator = Arc::new(
        crate::application::turn_coordinator::TurnCoordinator::with_event_spine_and_grok(
            kernel.clone(),
            Arc::new(canonical_store),
            canonical_event_spine.clone(),
            grok_hardening.clone(),
        )
        .with_event_projections(event_projections.clone())
        .with_backpressure(config.backpressure.clone())
        .with_session_input(session_input.clone()),
    );
    let workspace_checkpoint = Arc::new(
        crate::application::workspace_checkpoint::WorkspaceCheckpointService::new(
            Arc::new(
                crate::adapters::session::checkpoint_store_sqlite::SqliteCheckpointStore::open(
                    data_dir.join("workspace-checkpoints.sqlite"),
                )?,
            ),
            kernel.lease_manager(),
            grok_hardening.workspace_checkpoint,
        )
        .with_disk_quota(config.deployment.quotas.sessions_bytes)
        .with_safety_guard(agent_live_runs)
        .with_events(event_bus.clone(), Some(canonical_event_spine.clone())),
    );
    let session_service = Arc::new(
        crate::application::session_service::SessionService::with_protocol_journal(
            coordinator.store(),
            coordinator.active_index(),
            data_dir.join("protocol-events-v1.db"),
        )?,
    );
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
    let projection: Arc<dyn crate::application::post_turn_projection::PostTurnProjection> =
        Arc::new(
            crate::application::post_turn_projection::ProductionPostTurnProjection::new(
                crate::application::post_turn_projection::PostTurnProjectionResources {
                    corpus: domains.corpus(),
                    runtime: post_turn_runtime_port(runtime.clone(), domains.metacog()),
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
            active_profile: Arc::new(super::turn_runtime::ProductionActiveAgentProfile::new(
                active_profile.clone(),
                agent_profile_registry.clone(),
            )),
        },
    ));
    let lifecycle_registry =
        Arc::new(crate::application::lifecycle_contributors::LifecycleRegistry::default());
    let pipeline = Arc::new(crate::application::TurnPipeline::new(
        crate::application::turn_pipeline::TurnPipelineResources {
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
            conscious_core: Some(conscious_registry.clone()),
            session_input: session_input.clone(),
            prompt_queue_enabled: grok_hardening.prompt_queue,
            workspace_checkpoint: workspace_checkpoint.clone(),
            lifecycle: lifecycle_registry.clone(),
            lifecycle_enabled: grok_hardening.lifecycle_contributors,
            event_bus: event_bus.clone(),
        },
    ));
    let active_profile_port: Arc<
        dyn crate::application::turn_runtime_ports::ActiveAgentProfilePort,
    > = Arc::new(super::turn_runtime::ProductionActiveAgentProfile::new(
        active_profile.clone(),
        agent_profile_registry.clone(),
    ));
    let turn_orchestrator = Arc::new(crate::application::DaemonTurnOrchestrator::new(
        crate::application::daemon_turn::DaemonTurnResources {
            kernel: kernel.clone(),
            notify: shared_notify_tx.clone(),
            main_agent_process_id: main_agent_process_id.clone(),
            turn_token: turn_token.clone(),
            pipeline,
            coordinator,
            session_service: session_service.clone(),
            grok_hardening: grok_hardening.clone(),
            active_profile: active_profile_port,
        },
    ));

    let approved_apply = if pi_runtime.enabled && pi_work_allowed {
        Some(Arc::new(
            crate::application::approval::ApplyCoordinator::new(
                apply_objective_store,
                memory_group.approval_repository.clone(),
                kernel.clone(),
                clock.clone(),
                crate::application::approval::ApplyCoordinatorConfig {
                    worktree_base: pi_runtime.worktree_base.clone(),
                    timeout: std::time::Duration::from_secs(60),
                },
                Arc::new(crate::application::approval::GitManagedWorktreeCleaner),
            )?
            .with_memory_projection(
                crate::application::memory_projection::MemoryProjection::new(
                    canonical_event_spine.clone(),
                    event_projections.clone(),
                ),
            ),
        ))
    } else {
        None
    };

    Ok(TurnServices {
        session_input,
        session_gateway,
        turn_orchestrator,
        approved_apply,
        lifecycle_registry,
    })
}
