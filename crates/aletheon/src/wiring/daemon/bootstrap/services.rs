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

use super::session_infrastructure::SessionInfrastructure;

struct DurableRuntimeTurnEventSink {
    spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
}

struct DurableRuntimeAgentEventSink {
    spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
}

fn replay_runtime_agent_events(
    spine: &adapters_sqlite::event_spine::SqliteEventSpine,
) -> anyhow::Result<Vec<runtime::AgentStreamEvent>> {
    const MAX_RUNTIME_AGENT_EVENTS: u64 = 8_192;
    use runtime::event_spine::EventSpine;
    let through = spine.committed_watermark()?;
    let mut after = through.saturating_sub(MAX_RUNTIME_AGENT_EVENTS);
    let mut events = Vec::new();
    loop {
        let page = spine.read_committed_page(after, through, 256)?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|(position, _)| *position).unwrap_or(after);
        for (position, event) in page {
            if event.schema.0 == ::contracts::SchemaId::EVENT_RUNTIME_AGENT_V1 {
                let payload = event.envelope.payload;
                if let Ok(stream_event) =
                    serde_json::from_value::<runtime::AgentStreamEvent>(payload.clone())
                {
                    // The embedded Runtime sequence is scoped to one writer
                    // generation and may restart at one after a daemon
                    // restart.  The EventSpine row id is the durable global
                    // cursor, so resequence replay at that boundary before
                    // strict Runtime validation.
                    events.push(runtime::AgentStreamEvent::new(
                        position,
                        stream_event.into_event(),
                    ));
                } else if let Ok(runtime_event) =
                    serde_json::from_value::<runtime::RuntimeEvent>(payload)
                {
                    // Additive replay for events written before the envelope
                    // was introduced. The physical journal position supplies
                    // the durable global sequence for the compatibility event.
                    events.push(runtime::AgentStreamEvent::new(position, runtime_event));
                }
            }
        }
    }
    // The bounded replay prefix may begin after an Agent admission while
    // still containing its terminal.  There is no live Runtime state to
    // restore for that run, so do not feed an orphan terminal to the strict
    // lifecycle reducer and mistake the window boundary for corruption.
    let started = events
        .iter()
        .filter_map(|event| match event.event.as_ref() {
            runtime::RuntimeEvent::AgentRunAccepted { agent_run, .. }
            | runtime::RuntimeEvent::AgentRunStarted { agent_run, .. } => Some(agent_run.clone()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    events.retain(|event| match event.event.as_ref() {
        runtime::RuntimeEvent::AgentRunSettled { agent_run, .. } => started.contains(agent_run),
        _ => true,
    });
    Ok(events)
}

#[async_trait::async_trait]
impl runtime::AgentEventSink for DurableRuntimeAgentEventSink {
    async fn append(&self, event: runtime::AgentStreamEvent) -> Result<(), runtime::RuntimeError> {
        use runtime::event_spine::EventSpine;
        event
            .validate()
            .map_err(|_| runtime::RuntimeError::UnknownSchema)?;
        let runtime_event = event.event.as_ref();
        let (session, run, key) = match runtime_event {
            runtime::RuntimeEvent::AgentRunAccepted {
                session,
                agent_run,
                generation,
                backend,
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!("agent-accepted:{}:{generation:?}:{backend:?}", agent_run.0),
            ),
            runtime::RuntimeEvent::AgentRunStarted {
                session,
                agent_run,
                generation,
                ..
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!("agent-started:{}:{generation:?}", agent_run.0),
            ),
            runtime::RuntimeEvent::AgentRunSettled {
                session,
                agent_run,
                terminal,
                generation,
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!("agent-settled:{}:{generation:?}:{terminal:?}", agent_run.0),
            ),
            runtime::RuntimeEvent::AgentRunMessage {
                session,
                agent_run,
                kind,
                correlation,
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!("agent-message:{}:{kind}:{correlation:?}", agent_run.0),
            ),
            runtime::RuntimeEvent::AgentRunRecovery {
                session,
                agent_run,
                generation,
                decision,
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!("agent-recovery:{}:{generation:?}:{decision}", agent_run.0),
            ),
            runtime::RuntimeEvent::AgentRunMailbox {
                session,
                agent_run,
                delivery_id,
                kind,
                correlation,
                delivery,
            } => (
                session.0.clone(),
                agent_run.0.clone(),
                format!(
                    "agent-mailbox:{}:{delivery_id}:{kind}:{correlation:?}:{delivery:?}",
                    agent_run.0
                ),
            ),
            _ => return Err(runtime::RuntimeError::UnknownSchema),
        };
        let key = format!("{key}:{}:{}", event.sequence, event.digest);
        let payload = serde_json::to_value(&event).map_err(|_| runtime::RuntimeError::Internal)?;
        let event_id = runtime::EventId(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            key.as_bytes(),
        ));
        let mut envelope = ::contracts::EnvelopeV2::new(
            ::contracts::SchemaId::from(::contracts::SchemaId::EVENT_RUNTIME_AGENT_V1),
            ::contracts::EnvelopeV2Target(format!("runtime-agent:{run}")),
            ::contracts::EnvelopeV2Target(format!("session:{session}")),
            ::contracts::EnvelopeV2Delivery::Direct,
            ::contracts::NamespaceId(format!("session:{session}")),
            payload.clone(),
        );
        envelope.id = ::contracts::MessageId(event_id.0);
        self.spine
            .append(runtime::UnsequencedEvent {
                tree_id: runtime::EventTreeId::for_root_session(&session),
                event_id,
                parent: None,
                identity: runtime::EventIdentity {
                    root_session_id: session.clone(),
                    session_id: session,
                    agent_id: Some(run),
                },
                envelope,
                visibility: runtime::EventVisibility::Control,
                payload: runtime::EventPayload::Inline { value: payload },
            })
            .map(|_| ())
            .map_err(|_| runtime::RuntimeError::Internal)
    }
}

fn replay_runtime_turn_events(
    spine: &adapters_sqlite::event_spine::SqliteEventSpine,
) -> anyhow::Result<Vec<runtime::TurnStreamEvent>> {
    const MAX_RUNTIME_TURN_EVENTS: u64 = 8_192;
    use runtime::event_spine::EventSpine;
    let through = spine.committed_watermark()?;
    let mut after = through.saturating_sub(MAX_RUNTIME_TURN_EVENTS);
    let mut events = Vec::new();
    loop {
        let page = spine.read_committed_page(after, through, 256)?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|(position, _)| *position).unwrap_or(after);
        for (position, event) in page {
            if event.schema.0 == ::contracts::SchemaId::EVENT_RUNTIME_TURN_V1 {
                let payload = event.envelope.payload;
                if let Ok(stream_event) =
                    serde_json::from_value::<runtime::TurnStreamEvent>(payload.clone())
                {
                    // Runtime stream sequences are scoped to one writer
                    // generation.  They restart after daemon replacement;
                    // the EventSpine row id is the durable global cursor used
                    // for bounded restart replay and ordering validation.
                    events.push(runtime::TurnStreamEvent::new(
                        position,
                        stream_event.into_event(),
                    ));
                } else if let Ok(runtime_event) =
                    serde_json::from_value::<runtime::RuntimeEvent>(payload)
                {
                    events.push(runtime::TurnStreamEvent::new(position, runtime_event));
                }
            }
        }
    }
    // A bounded row-id window can contain a terminal for a turn whose start
    // is just before the window.  That terminal is already durable and there
    // is no in-memory turn to fence, so omit it from Runtime reducer replay;
    // feeding it to the strict writer would misclassify a bounded-window
    // boundary as a malformed stream.
    let started = events
        .iter()
        .filter_map(|event| match event.event.as_ref() {
            runtime::RuntimeEvent::TurnStarted { turn, .. } => Some(turn.clone()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    events.retain(|event| match event.event.as_ref() {
        runtime::RuntimeEvent::TurnSettled { turn, .. } => started.contains(turn),
        _ => true,
    });
    Ok(events)
}

#[async_trait::async_trait]
impl runtime::TurnEventSink for DurableRuntimeTurnEventSink {
    async fn append(&self, event: runtime::TurnStreamEvent) -> Result<(), runtime::RuntimeError> {
        use runtime::event_spine::EventSpine;
        event
            .validate()
            .map_err(|_| runtime::RuntimeError::UnknownSchema)?;
        let runtime_event = event.event.as_ref();
        let (session, key) = match runtime_event {
            runtime::RuntimeEvent::TurnStarted { session, turn } => {
                (session.0.clone(), format!("turn-started:{}", turn.0))
            }
            runtime::RuntimeEvent::TurnSettled {
                session,
                turn,
                terminal,
            } => (
                session.0.clone(),
                format!("turn-settled:{}:{terminal:?}", turn.0),
            ),
            runtime::RuntimeEvent::TurnObserved {
                session,
                turn,
                kind,
            } => (
                session.0.clone(),
                format!("turn-observed:{}:{kind}", turn.0),
            ),
            _ => return Err(runtime::RuntimeError::UnknownSchema),
        };
        let key = format!("{key}:{}:{}", event.sequence, event.digest);
        let payload = serde_json::to_value(&event).map_err(|_| runtime::RuntimeError::Internal)?;
        let event_id = runtime::EventId(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            key.as_bytes(),
        ));
        let mut envelope = ::contracts::EnvelopeV2::new(
            ::contracts::SchemaId::from(::contracts::SchemaId::EVENT_RUNTIME_TURN_V1),
            ::contracts::EnvelopeV2Target(format!("runtime-turn:{session}")),
            ::contracts::EnvelopeV2Target(format!("session:{session}")),
            ::contracts::EnvelopeV2Delivery::Direct,
            ::contracts::NamespaceId(format!("session:{session}")),
            payload.clone(),
        );
        envelope.id = ::contracts::MessageId(event_id.0);
        self.spine
            .append(runtime::UnsequencedEvent {
                tree_id: runtime::EventTreeId::for_root_session(&session),
                event_id,
                parent: None,
                identity: runtime::EventIdentity {
                    root_session_id: session.clone(),
                    session_id: session,
                    agent_id: None,
                },
                envelope,
                visibility: runtime::EventVisibility::Control,
                payload: runtime::EventPayload::Inline { value: payload },
            })
            .map(|_| ())
            .map_err(|_| runtime::RuntimeError::Internal)
    }
}

use super::request_ports::{post_turn_runtime_port, TurnRuntimeFacadePorts};
use crate::wiring::daemon::context_working_set::ContextWorkingSet;
use crate::wiring::daemon::DaemonConfig;
use crate::wiring::domain::{MemoryGroup, SecurityGroup, SessionGroup};
use adapters_sqlite::runtime_agent::SqliteAgentRunProjection;
use crate::config::GrokHardeningConfig;

/// Composition adapter that keeps the concrete Corpus transaction registry at
/// the bootstrap boundary while exposing only the Aletheon authority port to
/// application services.
pub(super) struct CorpusChangeTransactionAuthority(
    corpus::tools::tools::change_transaction::ChangeTransactionRegistry,
);

impl CorpusChangeTransactionAuthority {
    pub(super) fn new(
        registry: corpus::tools::tools::change_transaction::ChangeTransactionRegistry,
    ) -> Self {
        Self(registry)
    }
}

pub(super) async fn build_transaction_review_service(
    tools: &Arc<Mutex<corpus::tools::tools::ToolRegistry>>,
    data_dir: &std::path::Path,
) -> anyhow::Result<Arc<application::settlement::TransactionReviewService>> {
    let registry = tools
        .lock()
        .await
        .change_transactions()
        .context("built-in tool registry lacks change transaction authority")?;
    Ok(Arc::new(
        application::settlement::TransactionReviewService::new(
            Arc::new(CorpusChangeTransactionAuthority::new(registry)),
            Arc::new(
                adapters_sqlite::transaction_settlement::SqliteTransactionSettlementStore::open(
                    data_dir.join("transaction-settlements.sqlite"),
                )?,
            ),
        ),
    ))
}

#[async_trait::async_trait]
impl application::settlement::ChangeTransactionAuthority for CorpusChangeTransactionAuthority {
    async fn snapshot(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
    ) -> anyhow::Result<Option<::contracts::change_transaction::ChangeTransactionSnapshot>> {
        Ok(self.0.snapshot(transaction_id).await)
    }

    async fn accept(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<::contracts::change_transaction::ChangeTransactionSnapshot> {
        self.0
            .accept(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }

    async fn request_repair(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<::contracts::change_transaction::ChangeTransactionSnapshot> {
        self.0
            .request_repair(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }

    async fn rollback(
        &self,
        transaction_id: ::contracts::change_transaction::ChangeTransactionId,
        owner_session_id: &str,
        owner_agent: Option<::contracts::AgentToolContext>,
        root: &std::path::Path,
    ) -> anyhow::Result<::contracts::change_transaction::ChangeTransactionSnapshot> {
        self.0
            .rollback(transaction_id, owner_session_id, owner_agent, root)
            .await
            .map_err(|failure| anyhow::anyhow!(failure.summary))
    }
}

// ── Stage 1: agent control service ──────────────────────────────────────

pub(super) struct AgentServices {
    pub agent_control: Arc<dyn ::contracts::AgentControlPort>,
    pub agent_host: Arc<dyn crate::wiring::application::agent_control::AgentHostEffects>,
    pub agent_recovery: crate::wiring::application::agent_control::AgentRecoveryReport,
    /// Rich host/admin projection. Lifecycle writes are Runtime-first through
    /// `RuntimeAgentRunProjection`; callers must not depend on SQL authority.
    pub agent_repository: Arc<dyn crate::wiring::application::agent_control::AgentRunProjection>,
    pub canonical_event_spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
    pub event_projections: Arc<adapters_sqlite::projection_set::DefaultEventProjectionSet>,
    pub agent_live_runs: Arc<crate::wiring::application::agent_control::LiveAgentRuns>,
    pub capability_rollups:
        Arc<crate::wiring::application::capability_benchmark::CapabilityRollupProjectionSink>,
    pub role_workflow_factory:
        Option<Arc<agora::cognitive_role_workflow::RoleWorkflowFactory>>,
    pub runtime_agent_supervisor: Arc<runtime::RuntimeAgentSupervisor>,
    pub runtime_agent_backend: Arc<dyn runtime::DelegateBackend>,
}

/// Composition-only launcher binding. Runtime owns the catalog and lifecycle;
/// this value carries a concrete host adapter until the supervisor registers
/// it at the single daemon composition root.
pub(super) struct RuntimeLauncherBinding {
    pub id: ::contracts::RuntimeId,
    pub launcher: Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher>,
    pub manifest: Option<runtime::RuntimeManifest>,
}

pub(super) async fn build_agent_services(
    data_dir: &std::path::Path,
    kernel: Arc<kernel::KernelRuntime>,
    clock: Arc<dyn ::contracts::Clock>,
    cancel_token: CancellationToken,
    config: &DaemonConfig,
    corpus: Arc<dyn corpus::CorpusService>,
    runtime_bindings: Vec<RuntimeLauncherBinding>,
    pi_backend: Option<Arc<crate::wiring::adapters::runtime::PiDelegateBackend>>,
    tools: Arc<Mutex<corpus::tools::tools::ToolRegistry>>,
    agent_profiles_for_tools: HashMap<String, ::contracts::AgentProfile>,
    agent_profile_catalog: Arc<crate::wiring::adapters::runtime::AgentProfileRegistry>,
    runtime_profile_requirements: HashMap<
        ::contracts::AgentProfileId,
        Vec<::contracts::AgentRuntimeCapability>,
    >,
    granted_capabilities: Arc<tokio::sync::RwLock<Vec<::contracts::CapabilityId>>>,
    durable_memory: Arc<dyn mnemosyne::MemoryService>,
    agora: Arc<dyn agora::AgoraService>,
    canonical_event_spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
    event_projections: Arc<adapters_sqlite::projection_set::DefaultEventProjectionSet>,
) -> anyhow::Result<AgentServices> {
    let agent_state_root = data_dir.join("agents");
    std::fs::create_dir_all(&agent_state_root)?;
    let sql_agent_projection = Arc::new(
        SqliteAgentRunProjection::open(agent_state_root.join("agent_control.db"))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let agent_daemon_generation = format!("daemon:{}", uuid::Uuid::new_v4());
    let settlement_path = agent_state_root.join("agent_settlement.db");
    adapters_sqlite::SqliteSettlementReceiptStore::migrate(&settlement_path)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let settlement_receipts = Arc::new(
        adapters_sqlite::SqliteSettlementReceiptStore::open(&settlement_path)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    );
    let capability_rollups = Arc::new(
        crate::wiring::application::capability_benchmark::CapabilityRollupProjectionSink::open(
            data_dir.join("evaluation-rollups.db"),
        )?,
    );
    let runtime_agent_supervisor = Arc::new(
        runtime::RuntimeAgentSupervisor::new(runtime::DelegateBackendRegistry::new())
            .with_event_sink(Arc::new(DurableRuntimeAgentEventSink {
                spine: canonical_event_spine.clone(),
            })),
    );
    let replayed_agent_events = replay_runtime_agent_events(canonical_event_spine.as_ref())
        .context("replay Runtime Agent lifecycle events")?;
    // Replay lifecycle state without settling unresolved children yet. The
    // host projection below still needs to inspect process/checkpoint state
    // before choosing Resume/Finalize/Interrupt; eagerly fencing here would
    // make a checkpointed child unrecoverable after a daemon restart.
    let replayed_runtime_agents = runtime_agent_supervisor
        .replay_from_stream(replayed_agent_events)
        .await
        .context("replay Runtime Agent lifecycle stream")?;
    if replayed_runtime_agents > 0 {
        info!(
            replayed_runtime_agents,
            "Runtime Agent lifecycle stream replayed before host recovery"
        );
    }
    let agent_repository: Arc<dyn crate::wiring::application::agent_control::AgentRunProjection> =
        Arc::new(runtime::RuntimeAgentRunProjection::new(
            sql_agent_projection,
            runtime_agent_supervisor.clone(),
        ));
    let agent_facades = crate::wiring::application::agent_control::AgentHostAdapter::new_runtime_only(
        kernel.clone(),
        clock.clone(),
        agent_repository.clone(),
        Arc::new(
            runtime::BoundedAgentAdmission::with_budget(
                runtime::AgentAdmissionPolicy {
                    max_agents_per_root: config.agent_admission.max_agents_per_root,
                    max_running_agents: config.agent_admission.max_running_agents,
                    max_depth: config.agent_admission.max_depth,
                    max_queued_per_root: config.agent_admission.max_queued_per_root,
                    sibling_fairness_quantum: config.agent_admission.sibling_fairness_quantum,
                    root_max_tokens: config.agent_admission.root_max_tokens,
                    root_max_cost_micro: config.agent_admission.root_max_cost_micro,
                    max_child_tokens: config.agent_admission.max_child_tokens,
                    max_child_cost_micro: config.agent_admission.max_child_cost_micro,
                    max_storage_bytes: config.agent_admission.max_storage_bytes,
                    max_storage_items: config.agent_admission.max_storage_items,
                },
                kernel.budget_controller(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        ),
        canonical_event_spine.clone(),
        runtime_agent_supervisor.clone(),
    )
    .with_agent_profiles(agent_profiles_for_tools.clone())
    .with_agent_profile_resolver({
        let catalog = agent_profile_catalog.clone();
        Arc::new(move |profile_id: &::contracts::AgentProfileId| {
            catalog
                .resolve_by_name(&profile_id.0)
                .ok()
                .map(|resolved| resolved.profile)
        })
    })
    .with_runtime_profile_requirements(runtime_profile_requirements)
    .with_capability_history(capability_rollups.clone())
    .with_cognitive_task_admission(Arc::new(
        agora::cognitive_workspace::CognitiveWorkspaceCoordinator::new(agora.clone()),
    ))
    .with_budget_controller(kernel.budget_controller())
    .with_runtime_process_supervisor(Arc::new(
        kernel::process::controller::LinuxRuntimeProcessSupervisor,
    ))
    .with_event_spine(canonical_event_spine.clone())
    .with_event_projections(event_projections.clone())
    .with_lifecycle_hooks(Arc::new(corpus::CorpusAgentLifecycleHookSink(corpus)))
    .with_memory_vault(Arc::new(
        mnemosyne::AgentMemoryVault::open(agent_state_root.join("agent_memory.db"))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?,
    ))
    .with_durable_memory(durable_memory)
    .with_subagent_settlement(agent_daemon_generation.clone(), settlement_receipts)
    .into_facades();
    let agent_host = agent_facades.effects.clone();
    let agent_lifecycle = agent_facades.lifecycle.clone();
    let runtime_agent_backend = Arc::new(
        crate::wiring::application::agent_control::RuntimeObservedAgentBackend::compatibility(
            &agent_host,
        ),
    );
    runtime_agent_supervisor.set_observed_backend(runtime_agent_backend.clone());
    let has_native_runtime = runtime_bindings
        .iter()
        .any(|binding| binding.id.0 == crate::wiring::adapters::runtime::NATIVE_COGNIT_RUNTIME_ID);
    for binding in runtime_bindings {
        let backend_id = runtime::DelegateBackendId(binding.id.0.clone());
        let runtime_backend: Arc<dyn runtime::DelegateBackend> = Arc::new(
            crate::wiring::application::agent_control::RuntimeObservedAgentBackend::pinned(
                &agent_host,
                binding.launcher,
            ),
        );
        if let Some(manifest) = binding.manifest {
            runtime_agent_supervisor
                .register_backend_with_manifest(backend_id, runtime_backend, manifest)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        } else {
            runtime_agent_supervisor
                .register_backend(backend_id, runtime_backend)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
    }
    // E6 owns the Pi-specific binding. The resident Pi delegate is composed
    // directly into Runtime's catalog; there is no Pi entry in the Aletheon
    // launcher registry and therefore no second selection path.
    let pi_backend_id =
        runtime::DelegateBackendId(crate::wiring::adapters::runtime::PI_CODER_RUNTIME_ID.to_owned());
    if let Some(pi_backend) = pi_backend {
        pi_backend
            .bind_host(&agent_host, &pi_backend)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        runtime_agent_supervisor
            .register_backend_with_manifest(
                pi_backend_id,
                pi_backend,
                crate::wiring::adapters::runtime::pi_manifest().clone(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    }
    let agent_recovery = agent_lifecycle
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
    let agent_cleanup = crate::wiring::application::agent_control::AgentCleanupCoordinator::new(
        agent_repository.clone(),
        Arc::new(
            crate::wiring::adapters::runtime::worktree_recovery::VerifiedAgentWorktreeReclaimer::default(),
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
    let agent_control = agent_facades.control.clone();
    let role_workflow_factory = if has_native_runtime {
        let profiles =
            super::role_profiles::resolve_role_launch_profiles(&agent_profiles_for_tools)?;
        Some(Arc::new(
            agora::cognitive_role_workflow::RoleWorkflowFactory::new(
                agent_control.clone(),
                Arc::new(
                    agora::cognitive_workspace::CognitiveWorkspaceCoordinator::new(agora.clone()),
                ),
                ::contracts::RuntimeId(
                    crate::wiring::adapters::runtime::NATIVE_COGNIT_RUNTIME_ID.into(),
                ),
                profiles,
                10 * 60 * 1_000,
            )?,
        ))
    } else {
        None
    };
    let agent_live_runs = agent_lifecycle.live_runs();
    let agent_shutdown_cancel = cancel_token.clone();
    let agent_lifecycle_for_shutdown = agent_lifecycle.clone();
    tokio::spawn(async move {
        agent_shutdown_cancel.cancelled().await;
        agent_lifecycle_for_shutdown.shutdown().await;
    });

    super::runtime::register_agent_tools(
        tools.clone(),
        agent_control.clone(),
        agent_profiles_for_tools,
    )
    .await;
    *granted_capabilities.write().await = corpus::discover_tool_extensions(&tools)
        .await?
        .into_iter()
        .flat_map(|entry| entry.capabilities)
        .collect();

    Ok(AgentServices {
        agent_control,
        agent_host,
        agent_recovery,
        agent_repository,
        canonical_event_spine,
        event_projections,
        agent_live_runs,
        capability_rollups,
        role_workflow_factory,
        runtime_agent_supervisor,
        runtime_agent_backend,
    })
}

// ── Stage 2: session infrastructure + turn pipeline ─────────────────────

pub(super) struct TurnServices {
    pub session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub session_gateway: Arc<dyn crate::wiring::daemon::handler::ports::SessionProjectionPort>,
    pub session_memory: Arc<dyn crate::wiring::daemon::handler::ports::SessionMemoryProjectionPort>,
    pub turn_orchestrator: Arc<crate::wiring::application::DaemonTurnOrchestrator>,
    pub approved_apply: Option<Arc<crate::wiring::application::approval::ApplyCoordinator>>,
    pub lifecycle_registry: Arc<runtime::lifecycle_contributors::LifecycleRegistry>,
    pub evaluation_service: Arc<crate::wiring::application::evaluation::EvaluationService>,
    pub workspace_checkpoint:
        Arc<crate::wiring::application::workspace_checkpoint::WorkspaceCheckpointService>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_turn_services(
    data_dir: &std::path::Path,
    kernel: Arc<kernel::KernelRuntime>,
    clock: Arc<dyn ::contracts::Clock>,
    cancel_token: CancellationToken,
    event_bus: Option<Arc<runtime::event_projection::CanonicalEventBus>>,
    config: &DaemonConfig,
    grok_hardening: GrokHardeningConfig,
    evaluation: crate::config::EvaluationSettings,
    pi_runtime: &crate::config::CodingRuntimeConfig,
    pi_work_allowed: bool,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<ContextWorkingSet>>>>>,
    session_id: &str,
    initial_session: Arc<Mutex<ContextWorkingSet>>,
    core_memory: Arc<Mutex<mnemosyne::runtime::CoreMemory>>,
    recall_memory: Arc<Mutex<mnemosyne::runtime::RecallMemory>>,
    self_field: Arc<Mutex<dasein::SelfField>>,
    llm: Arc<dyn ::contracts::LlmProvider>,
    debug_perf: Arc<kernel::debug_bus::PerfCounter>,
    model_router: Arc<super::super::model_router::ModelRouter>,
    domains: &crate::wiring::domain::DomainServices,
    security_group: &SecurityGroup,
    memory_group: &MemoryGroup,
    memory_gateway: Arc<mnemosyne::MemoryGatewayService>,
    session_group: &SessionGroup,
    capability_resources: crate::wiring::daemon::handler::tool_executor::CapabilityResources,
    conscious_registry: Arc<
        crate::wiring::application::conscious_workspace::ConsciousWorkspaceRegistry,
    >,
    context_assembler: Arc<crate::wiring::application::context_assembler::ContextAssembler>,
    apply_objective_store: Arc<std::sync::Mutex<crate::wiring::application::goal::ObjectiveStore>>,
    agent_live_runs: Arc<crate::wiring::application::agent_control::LiveAgentRuns>,
    capability_rollups: Arc<
        crate::wiring::application::capability_benchmark::CapabilityRollupProjectionSink,
    >,
    role_workflow_factory: Option<
        Arc<agora::cognitive_role_workflow::RoleWorkflowFactory>,
    >,
    canonical_event_spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
    event_projections: Arc<adapters_sqlite::projection_set::DefaultEventProjectionSet>,
    agent_profile_registry: Arc<crate::wiring::adapters::runtime::AgentProfileRegistry>,
    active_profile: Arc<Mutex<String>>,
    runtime: Arc<Mutex<crate::wiring::cognitive_runtime::AletheonCognitiveRuntime>>,
    turn_token: Arc<Mutex<Option<CancellationToken>>>,
    main_agent_process_ids: Arc<Mutex<std::collections::HashMap<String, ::contracts::ProcessId>>>,
    approval_owner_process_id: Arc<Mutex<Option<::contracts::ProcessId>>>,
    session_infra: &SessionInfrastructure,
) -> anyhow::Result<TurnServices> {
    // The infrastructure is composed once by RequestHandler bootstrap. Every
    // turn service, coordinator and recovery scan receives these same handles;
    // this function is not a composition root and must not reopen a store.
    let session_store = session_infra.append_store();
    let session_commands = session_infra.writer();
    let shared_notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>> = Arc::new(Mutex::new(None));
    let session_id = session_id.to_owned();

    let turn_recovery_report = runtime::turn_recovery::scan_incomplete_turns(
        session_store.as_ref(),
        grok_hardening.compaction_v2,
    )
    .await
    .context("incomplete-turn recovery scan during daemon startup")?;
    runtime::turn_recovery::persist_recovery_health(data_dir, &turn_recovery_report)
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
        let coordinator = application::session_input::SessionInputCoordinator::new(Arc::new(
            adapters_sqlite::prompt_queue::SqlitePromptQueueStore::open(
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
        Arc::new(application::session_input::SessionInputCoordinator::in_memory())
    };
    let memory_evaluation_projection =
        crate::wiring::application::memory_projection::MemoryProjection::new(
            canonical_event_spine.clone(),
            event_projections.clone(),
        );
    let mut evaluation_sinks: Vec<
        Arc<dyn application::evaluation_projection::EvaluationProjectionSink>,
    > = vec![
        Arc::new(
            crate::wiring::application::goal::GoalEvaluationProjectionSink::new(
                canonical_event_spine.clone(),
                apply_objective_store.clone(),
            ),
        ),
        Arc::new(
            crate::wiring::application::agent_control::settlement::AgentEvaluationProjectionSink::new(
                canonical_event_spine.clone(),
            ),
        ),
        Arc::new(
            crate::wiring::application::memory_projection::MemoryEvaluationProjectionSink::new(
                memory_evaluation_projection,
            ),
        ),
        Arc::new(agora::cognitive_workspace::AgoraEvaluationProjectionSink::new(domains.agora())),
        capability_rollups.clone(),
    ];
    if let Some(dasein) = self_field.lock().await.dasein_handle() {
        evaluation_sinks.push(Arc::new(
            crate::wiring::composition::dasein_workspace::DaseinEvaluationProjectionSink::new(
                dasein,
                clock.clone(),
            ),
        ));
    }
    let evaluation_projection =
        Arc::new(application::evaluation_projection::EvaluationProjection::new(evaluation_sinks));
    let evaluation_service =
        crate::wiring::composition::turn_coordinator::compose_evaluation_service(
            kernel.clone(),
            data_dir,
            evaluation,
            evaluation_projection,
            capability_rollups.clone(),
        )?;
    let runtime_turn_writer = Arc::new(runtime::RuntimeTurnWriter::new().with_event_sink(
        Arc::new(DurableRuntimeTurnEventSink {
            spine: canonical_event_spine.clone(),
        }),
    ));
    let replayed_turn_events = replay_runtime_turn_events(canonical_event_spine.as_ref())
        .context("replay Runtime turn lifecycle events")?;
    let mut previous_runtime_turn_sequence = 0;
    for (index, event) in replayed_turn_events.iter().enumerate() {
        event.validate().map_err(|reason| {
            anyhow::anyhow!(
                "Runtime turn replay event {index} (sequence {}) failed validation: {reason}",
                event.sequence
            )
        })?;
        if event.sequence <= previous_runtime_turn_sequence {
            anyhow::bail!(
                "Runtime turn replay sequence regressed at event {index}: {} after {}",
                event.sequence,
                previous_runtime_turn_sequence
            );
        }
        previous_runtime_turn_sequence = event.sequence;
    }
    let recovered_runtime_turns = runtime_turn_writer
        .recover_from_stream(replayed_turn_events)
        .await
        .context("replay Runtime turn lifecycle stream")?;
    if recovered_runtime_turns > 0 {
        info!(
            recovered_runtime_turns,
            "Runtime turn restart recovery fenced orphaned turns"
        );
    }
    let coordinator = Arc::new(
        crate::wiring::application::turn_coordinator::TurnCoordinator::from_components_with_runtime_turn_writer(
            kernel.clone(),
            session_store,
            grok_hardening.clone(),
            runtime_turn_writer,
        )
        .with_backpressure(config.backpressure.clone())
        .with_canonical_session_guard(true)
        .with_session_input(session_input.clone())
        .with_evaluation_service(evaluation_service.clone())
        .with_host_acceptance(Arc::new(
            agora::host_acceptance::HostAcceptanceController::new(domains.agora()),
        )),
    );
    let workspace_checkpoint = Arc::new(
        crate::wiring::application::workspace_checkpoint::WorkspaceCheckpointService::new(
            Arc::new(
                crate::wiring::adapters::session::checkpoint_store_sqlite::SqliteCheckpointStore::open(
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
        runtime::session_service::SessionService::with_protocol_journal(
            coordinator.store(),
            coordinator.active_index(),
            data_dir.join("protocol-events-v1.db"),
        )?
        .with_runtime_commands(session_commands),
    );
    if let Some(replay) = session_service
        .try_resume(&::contracts::SessionId(session_id.clone()))
        .await?
    {
        initial_session
            .lock()
            .await
            .restore_messages(replay.messages);
    }
    let session_gateway = Arc::new(
        crate::wiring::daemon::session_projection::CanonicalSessionProjection::new(
            session_service.clone(),
        ),
    );
    let session_memory = Arc::new(
        crate::wiring::daemon::session_projection::CanonicalSessionMemoryProjection::new(
            core_memory,
            recall_memory,
        ),
    );
    let projection: Arc<dyn crate::wiring::application::post_turn_projection::PostTurnProjection> =
        Arc::new(
            crate::wiring::application::post_turn_projection::ProductionPostTurnProjection::new(
                crate::wiring::application::post_turn_projection::PostTurnProjectionResources {
                    corpus: domains.corpus(),
                    runtime: post_turn_runtime_port(
                        runtime.clone(),
                        domains.metacog(),
                        self_field.clone(),
                        clock.clone(),
                        Arc::new(
                            crate::wiring::composition::evolution_proposer::GovernedEvolutionProposer::new(
                                apply_objective_store.clone(),
                                memory_group.approval_repository.clone(),
                                capability_rollups.clone(),
                                clock.clone(),
                                data_dir.join("evolution-proposals.db"),
                            )?,
                        ),
                    ),
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
            agent_admission: config.agent_admission.clone(),
            performance: debug_perf.clone(),
        },
    ));
    let lifecycle_registry =
        Arc::new(runtime::lifecycle_contributors::LifecycleRegistry::default());
    let active_profile_port: Arc<
        dyn crate::wiring::application::turn_runtime_ports::ActiveAgentProfilePort,
    > = Arc::new(super::turn_runtime::ProductionActiveAgentProfile::new(
        active_profile.clone(),
        agent_profile_registry.clone(),
    ));
    let pipeline = Arc::new(crate::wiring::application::TurnPipeline::new(
        crate::wiring::application::turn_pipeline::TurnPipelineResources {
            notify: shared_notify_tx.clone(),
            clock: clock.clone(),
            agora: Some(domains.agora()),
            kernel: kernel.clone(),
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
            role_workflow_factory,
            active_profile: active_profile_port.clone(),
            memory_gateway,
        },
    ));
    let turn_orchestrator = Arc::new(crate::wiring::application::DaemonTurnOrchestrator::compose(
        kernel.clone(),
        shared_notify_tx.clone(),
        main_agent_process_ids.clone(),
        approval_owner_process_id.clone(),
        turn_token.clone(),
        pipeline,
        coordinator,
        session_service.clone(),
        grok_hardening.clone(),
        active_profile_port,
    ));

    let approved_apply = if pi_runtime.enabled && pi_work_allowed {
        Some(Arc::new(
            crate::wiring::application::approval::ApplyCoordinator::new(
                apply_objective_store,
                memory_group.approval_repository.clone(),
                kernel.clone(),
                clock.clone(),
                crate::wiring::application::approval::ApplyCoordinatorConfig {
                    worktree_base: pi_runtime.worktree_base.clone(),
                    timeout: std::time::Duration::from_secs(60),
                },
                Arc::new(crate::wiring::application::approval::GitManagedWorktreeCleaner),
            )?
            .with_memory_projection(
                crate::wiring::application::memory_projection::MemoryProjection::new(
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
        session_memory,
        turn_orchestrator,
        approved_apply,
        lifecycle_registry,
        evaluation_service,
        workspace_checkpoint,
    })
}
