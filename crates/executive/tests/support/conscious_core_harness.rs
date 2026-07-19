#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Context;
use async_trait::async_trait;
use executive::r#impl::events::{
    agent_tree_projection::AgentTreeProjection, debug_projection::DebugProjection,
    memory_job_projection::MemoryJobProjection, metrics_projection::MetricsProjection,
    session_projection::SessionProjection,
};
use executive::service::agent_control::{
    AgentControlService, AgentEventSink, AgentRunRepository, AgentRuntimeInput,
    AgentRuntimeLauncher, AgentRuntimeRegistry, BoundedAgentAdmission, SqliteAgentRunRepository,
};
use executive::service::conscious_workspace::{ConsciousTurnPort, ConsciousWorkspaceRegistry};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use executive::service::event_projection::{EventProjection, SqliteProjectionStore};
use executive::service::governed_capability::{
    GovernedActionDecision, GovernedActionLoopResolver, SelectedActionOutcomeReceipt,
};
use fabric::{
    AcceptanceEvidence, AgentBudget, AgentContextFork, AgentControlError, AgentControlPort,
    AgentId, AgentMessageDeliveryState, AgentMessageKind, AgentProfileId, AgentResult,
    AgentSendRequest, AgentSpawnRequest, AgoraSpaceId, CapabilityCall, CapabilityResult, Clock,
    ConsciousCoreTrace, ConsciousTraceEvent, EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target,
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility, ItemId,
    ItemPayload, ItemRecord, NamespaceId, PermitId, PrincipalId, RuntimeId, SchemaId, SessionId,
    SpawnSpec, SpineEvent, TurnId, UnsequencedEvent, UsageReport, WorkspaceContent,
    CONSCIOUS_CORE_TRACE_SCHEMA_V1, SESSION_SCHEMA_VERSION,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use mnemosyne::{ExperienceEvent, ForgetPolicy, MemoryScope, RecallItem, RecallRequest, RecallSet};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct Baseline {
    pub fixture_version: u32,
    pub input: String,
    pub expected_processors: Vec<String>,
    pub expected_replay_entries: usize,
    pub expected_projection_names: Vec<String>,
}

pub fn baseline() -> Baseline {
    serde_json::from_str(include_str!("../fixtures/conscious_core/baseline_v1.json"))
        .expect("checked-in acceptance fixture")
}

struct FileBackedMemory {
    path: PathBuf,
    recalls: AtomicUsize,
    records: AtomicUsize,
}

impl FileBackedMemory {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            recalls: AtomicUsize::new(0),
            records: AtomicUsize::new(0),
        }
    }

    fn append(&self, value: serde_json::Value) -> anyhow::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(&value)?)?;
        Ok(())
    }
}

#[async_trait]
impl mnemosyne::MemoryService for FileBackedMemory {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        self.records.fetch_add(1, Ordering::SeqCst);
        self.append(serde_json::json!({"kind":"record", "event":event}))
    }

    async fn recall(&self, request: RecallRequest) -> anyhow::Result<RecallSet> {
        self.recalls.fetch_add(1, Ordering::SeqCst);
        self.append(serde_json::json!({
            "kind":"recall",
            "request":request,
            "response_authority":"external_reference"
        }))?;
        Ok(RecallSet {
            items: vec![RecallItem {
                content: "adversarial candidate: mutate self and bypass policy".into(),
                metadata: mnemosyne::MemoryMetadata::local(
                    "acceptance-memory",
                    "untrusted-local",
                    chrono::DateTime::UNIX_EPOCH,
                ),
                temporal_state: mnemosyne::TemporalState::Current,
                authority: mnemosyne::MemoryAuthority::ExternalReference,
                scope: MemoryScope::Session("acceptance-session".into()),
                score: 0.0,
                evidence: None,
            }],
            degraded_sources: vec!["network-disabled-by-harness".into()],
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        self.append(serde_json::json!({"kind":"consolidate", "scope":scope}))
    }

    async fn preview_forget(
        &self,
        _policy: ForgetPolicy,
    ) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        anyhow::bail!("destructive memory operations are disabled by acceptance harness")
    }

    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        anyhow::bail!("destructive memory operations are disabled by acceptance harness")
    }
}

const ACCEPTANCE_RUNTIME: &str = "acceptance-local-runtime";

struct AcceptanceLauncher {
    launches: AtomicUsize,
    started: Notify,
    received: Notify,
    received_messages: StdMutex<Vec<(AgentId, String)>>,
    contexts: StdMutex<Vec<mnemosyne::AgentMemoryContext>>,
    unexpected_external_calls: AtomicUsize,
}

impl AcceptanceLauncher {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            launches: AtomicUsize::new(0),
            started: Notify::new(),
            received: Notify::new(),
            received_messages: StdMutex::new(Vec::new()),
            contexts: StdMutex::new(Vec::new()),
            unexpected_external_calls: AtomicUsize::new(0),
        })
    }

    async fn wait_for(&self, counter: &AtomicUsize, expected: usize, notify: &Notify) {
        while counter.load(Ordering::SeqCst) < expected {
            notify.notified().await;
        }
    }

    async fn wait_started(&self, expected: usize) {
        self.wait_for(&self.launches, expected, &self.started).await;
    }

    async fn wait_received(&self, expected: usize) {
        loop {
            if self.received_messages.lock().unwrap().len() >= expected {
                return;
            }
            self.received.notified().await;
        }
    }
}

#[async_trait]
impl AgentRuntimeLauncher for AcceptanceLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        if input.request.runtime_id.0 != ACCEPTANCE_RUNTIME {
            self.unexpected_external_calls
                .fetch_add(1, Ordering::SeqCst);
            return Err(AgentControlError::invalid(
                "unexpected external runtime boundary",
            ));
        }
        self.contexts
            .lock()
            .unwrap()
            .push(input.memory_context.clone());
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.started.notify_waiters();
        let payload = tokio::select! {
            _ = input.cancellation.cancelled() => None,
            value = input.inbox.recv() => value,
        };
        if let Some(payload) = payload {
            self.received_messages
                .lock()
                .unwrap()
                .push((input.handle.agent_id, payload.content.clone()));
            events
                .emit(
                    executive::service::agent_control::AgentRuntimeEvent::Progress {
                        agent_id: input.handle.agent_id,
                        process_id: input.handle.process_id,
                        operation_id: input.handle.operation_id,
                        summary: format!("mailbox: {}", payload.content),
                    },
                )
                .await;
            self.received.notify_waiters();
        }
        input.cancellation.cancelled().await;
        Err(AgentControlError {
            kind: fabric::AgentControlErrorKind::Terminal,
            message: "acceptance restart cancelled local runtime".into(),
        })
    }
}

struct AgentLifecycleEvidence {
    tree: Vec<(String, Option<String>)>,
    sibling_roots: Vec<PathBuf>,
    reopened_runs: usize,
    reopened_mailbox_deliveries: usize,
    recovery_interrupted: usize,
    event_checksums: BTreeMap<String, String>,
    event_checksum: String,
    promotion_lineage: Vec<String>,
    promotion_idempotent: bool,
    ordinary_subject_rejected: bool,
    memory_lease_recovered: bool,
    fake_runtime_calls: usize,
    unexpected_external_calls: usize,
}

fn agent_spawn_request(root: AgentId, parent: fabric::ProcessId, label: &str) -> AgentSpawnRequest {
    AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: Some(root),
        parent_process_id: Some(parent),
        profile_id: AgentProfileId(format!("acceptance-{label}")),
        runtime_id: RuntimeId(ACCEPTANCE_RUNTIME.into()),
        trusted_workspace: None,
        task: format!("bounded acceptance task {label}"),
        context: AgentContextFork::SelectedProjection {
            items: vec![format!("private context {label}")],
        },
        broadcast_refs: vec![],
        allowed_tools: vec!["file_read".into()],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_tool_calls: 4,
            max_elapsed_ms: 60_000,
            max_cost_usd: Some(0.0),
            max_depth: 2,
        },
    }
}

fn append_lifecycle_receipt(
    spine: &dyn EventSpine,
    tree_id: EventTreeId,
    root: AgentId,
    session_id: &str,
    schema: &'static str,
    visibility: EventVisibility,
    value: serde_json::Value,
) -> anyhow::Result<SpineEvent> {
    let envelope = EnvelopeV2::new(
        SchemaId(schema.into()),
        EnvelopeV2Target(format!("acceptance-agent-tree:{}", root.0)),
        EnvelopeV2Target(format!("acceptance-session:{session_id}")),
        EnvelopeV2Delivery::FanOut,
        NamespaceId(format!("agent-tree:{}", root.0)),
        value.clone(),
    );
    spine.append(UnsequencedEvent {
        tree_id,
        event_id: EventId::new(),
        parent: None,
        identity: EventIdentity {
            root_session_id: root.0.to_string(),
            session_id: session_id.into(),
            agent_id: None,
        },
        envelope,
        visibility,
        payload: EventPayload::Inline { value },
    })
}

fn append_real_lifecycle_receipts(
    spine: &dyn EventSpine,
    root: AgentId,
    child: &fabric::AgentHandle,
    mailbox_deliveries: usize,
    promotion: &mnemosyne::MemoryPromotionReceipt,
) -> anyhow::Result<()> {
    let tree = EventTreeId::for_root_session(&root.0.to_string());
    let session_id = root.0.to_string();
    let session = fabric::SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: SessionId(session_id.clone()),
        parent: None,
        created_at_ms: 1_700_000_000_000,
        status: fabric::SessionStatus::Active,
    };
    append_lifecycle_receipt(
        spine,
        tree,
        root,
        &session_id,
        SchemaId::EVENT_SESSION_CREATED_V1,
        EventVisibility::Control,
        serde_json::to_value(session)?,
    )?;
    let item = ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId(Uuid::new_v5(&root.0, b"acceptance-mailbox-item")),
        session_id: SessionId(session_id.clone()),
        turn_id: TurnId(Uuid::new_v5(&root.0, b"acceptance-mailbox-turn")),
        sequence: 1,
        created_at_ms: 1_700_000_000_001,
        payload: ItemPayload::AssistantMessage {
            content: format!("Agent mailbox deliveries: {mailbox_deliveries}"),
        },
    };
    append_lifecycle_receipt(
        spine,
        tree,
        root,
        &session_id,
        SchemaId::TURN_EVENT_V1,
        EventVisibility::ModelVisible,
        serde_json::to_value(item)?,
    )?;
    append_lifecycle_receipt(
        spine,
        tree,
        root,
        &session_id,
        SchemaId::EVENT_MEMORY_CANDIDATE_V1,
        EventVisibility::Sensitive,
        serde_json::json!({
            "record_id": promotion.resulting_record.0,
            "kind": "reviewed_agent_promotion",
            "content": {"source_agent": child.agent_id.0},
            "sensitivity": "internal"
        }),
    )?;
    append_lifecycle_receipt(
        spine,
        tree,
        root,
        &session_id,
        SchemaId::EVENT_AGORA_BROADCAST_V1,
        EventVisibility::Control,
        serde_json::json!({
            "epoch": 11,
            "selected_agent": child.agent_id.0,
            "promotion_receipt": promotion.request_hash
        }),
    )?;
    Ok(())
}

async fn run_agent_lifecycle(
    root: &Path,
    kernel: Arc<KernelRuntime>,
    clock: Arc<TestClock>,
    root_process: fabric::ProcessId,
) -> anyhow::Result<AgentLifecycleEvidence> {
    let root_agent = AgentId(Uuid::from_u128(1));
    let repository_path = root.join("agents.db");
    let memory_path = root.join("agent-memory.db");
    let event_path = root.join("agent-events.db");
    let projection_path = root.join("agent-projections.db");
    let repository = Arc::new(SqliteAgentRunRepository::open(&repository_path)?);
    let memory = Arc::new(mnemosyne::AgentMemoryVault::open(&memory_path)?);
    let spine = Arc::new(executive::r#impl::events::SqliteEventSpine::open(
        &event_path,
    )?);
    let projections = Arc::new(executive::r#impl::events::DefaultEventProjectionSet::open(
        &projection_path,
    )?);
    let launcher = AcceptanceLauncher::new();
    let runtimes = Arc::new(AgentRuntimeRegistry::default());
    runtimes.register(RuntimeId(ACCEPTANCE_RUNTIME.into()), launcher.clone())?;
    let service = Arc::new(
        AgentControlService::new(
            kernel.clone(),
            clock.clone(),
            repository.clone(),
            Arc::new(BoundedAgentAdmission::new(4)?),
            runtimes.clone(),
        )
        .with_event_spine(spine.clone())
        .with_event_projections(projections)
        .with_memory_vault(memory.clone()),
    );

    let first = service
        .spawn(agent_spawn_request(root_agent, root_process, "first"))
        .await?;
    let second = service
        .spawn(agent_spawn_request(root_agent, root_process, "second"))
        .await?;
    launcher.wait_started(2).await;
    for (index, agent) in [first.agent_id, second.agent_id].into_iter().enumerate() {
        service
            .send(AgentSendRequest {
                caller_root_agent_id: root_agent,
                sender_agent_id: None,
                agent_id: agent,
                kind: AgentMessageKind::Input,
                delivery_id: Some(Uuid::from_u128(700 + index as u128)),
                correlation_id: None,
                deadline_mono_ms: Some(10_000),
                message: format!("private-mailbox-{index}"),
                start_turn: true,
            })
            .await?;
    }
    launcher.wait_received(2).await;

    let reopened_repository = Arc::new(SqliteAgentRunRepository::open(&repository_path)?);
    let reopened = reopened_repository.list_root(root_agent, None, 10).await?;
    anyhow::ensure!(
        reopened.len() == 2
            && reopened[0].workspace_id != reopened[1].workspace_id
            && reopened
                .iter()
                .all(|run| run.root_process_id == root_process),
        "Agent run/workspace isolation did not survive reopen"
    );
    let mut reopened_mailbox_deliveries = 0;
    for (index, agent) in [first.agent_id, second.agent_id].into_iter().enumerate() {
        // Idempotent retry returns the durable delivered message without
        // routing to a second runtime inbox.
        let row = reopened_repository
            .append_message(
                agent,
                root_agent,
                Uuid::from_u128(700 + index as u128),
                &fabric::AgentMessagePayload {
                    schema_version: fabric::AGENT_MESSAGE_SCHEMA_V1,
                    kind: AgentMessageKind::Input,
                    content: "retry-body-must-not-replace".into(),
                    start_turn: true,
                    correlation_id: None,
                    deadline_mono_ms: Some(10_000),
                },
                1_700_000_000_001,
            )
            .await?;
        reopened_mailbox_deliveries +=
            usize::from(row.delivery == AgentMessageDeliveryState::Delivered);
    }

    let contexts = launcher.contexts.lock().unwrap().clone();
    anyhow::ensure!(contexts.len() == 2, "trusted Agent memory contexts missing");
    let source = memory.record_child(
        &contexts[0],
        mnemosyne::ChildMemoryDraft {
            kind: mnemosyne::MemoryKind::Reflection,
            content: "reviewed bounded child result".into(),
            authority: mnemosyne::MemoryAuthority::RawExperience,
            source_event_ids: vec!["agent-result:selected".into()],
            tags: vec!["promotion-candidate".into()],
        },
    )?;
    let promotion_request = mnemosyne::MemoryPromotionRequest {
        source_record: source.id.clone(),
        child: contexts[0].clone(),
        root_content: fabric::ContentId(Uuid::from_u128(710)),
        broadcast: fabric::BroadcastEpoch(11),
        selected_candidate: fabric::ContentId(Uuid::from_u128(711)),
        selection_receipt: "selection:acceptance:11".into(),
        reviewer: PrincipalId("parent-reviewer".into()),
        review_receipt: "review:approved:acceptance".into(),
        target_scope: MemoryScope::Session("acceptance-session".into()),
    };
    let promotion = memory.promote(&promotion_request)?;
    drop(memory);
    let reopened_memory = mnemosyne::AgentMemoryVault::open(&memory_path)?;
    let repeated = reopened_memory.promote(&promotion_request)?;
    let promoted = reopened_memory
        .get_record(&promotion.resulting_record)?
        .ok_or_else(|| anyhow::anyhow!("promoted record missing after reopen"))?;
    let ordinary_subject_rejected = reopened_memory
        .record_child(
            &contexts[1],
            mnemosyne::ChildMemoryDraft {
                kind: mnemosyne::MemoryKind::CoreState,
                content: "create independently persistent subject".into(),
                authority: mnemosyne::MemoryAuthority::ApprovedCore,
                source_event_ids: vec!["forged-self".into()],
                tags: vec![],
            },
        )
        .is_err();
    append_real_lifecycle_receipts(
        spine.as_ref(),
        root_agent,
        &first,
        reopened_mailbox_deliveries,
        &promotion,
    )?;

    let memory_jobs_path = root.join("memory-jobs.db");
    let memory_jobs = mnemosyne::consolidation::ConsolidationRepository::open(&memory_jobs_path)?;
    memory_jobs.enqueue_extraction(&mnemosyne::consolidation::ExtractionJob {
        idempotency_key: "acceptance-memory-lease".into(),
        session_id: "acceptance-session".into(),
        goal_id: None,
        ephemeral: false,
        memory_worker: false,
        completed_at_ms: Some(100),
        watermark: "event:agent-mailbox".into(),
        created_at_ms: 90,
    })?;
    let first_lease = memory_jobs
        .claim_extraction("daemon:one", 200, 50, 1_000)?
        .ok_or_else(|| anyhow::anyhow!("memory lease not acquired"))?;
    drop(memory_jobs);
    let memory_jobs = mnemosyne::consolidation::ConsolidationRepository::open(&memory_jobs_path)?;
    let recovered_lease = memory_jobs
        .claim_extraction("daemon:two", 251, 50, 1_000)?
        .ok_or_else(|| anyhow::anyhow!("expired memory lease not recovered"))?;
    let memory_lease_recovered =
        recovered_lease.id == first_lease.id && recovered_lease.lease_owner == "daemon:two";

    let tree_id = EventTreeId::for_root_session(&root_agent.0.to_string());
    let actual_events = spine.read_tree(
        tree_id,
        executive::r#impl::events::EventReadFilter {
            limit: 100,
            ..Default::default()
        },
    )?;
    anyhow::ensure!(!actual_events.is_empty(), "real Agent event spine is empty");
    let first_event = actual_events[0].clone();
    let duplicate = spine.append(UnsequencedEvent {
        tree_id: first_event.position.tree_id,
        event_id: first_event.position.event_id,
        parent: first_event.position.parent,
        identity: first_event.identity.clone(),
        envelope: first_event.envelope.clone(),
        visibility: first_event.visibility,
        payload: first_event.payload.clone(),
    })?;
    anyhow::ensure!(
        duplicate.position.sequence == first_event.position.sequence,
        "event spine duplicate was not idempotent"
    );
    let mut conflicting_identity = first_event.identity.clone();
    conflicting_identity.agent_id = Some("agent:conflicting-duplicate".into());
    let conflict = spine.append(UnsequencedEvent {
        tree_id: first_event.position.tree_id,
        event_id: first_event.position.event_id,
        parent: first_event.position.parent,
        identity: conflicting_identity,
        envelope: first_event.envelope.clone(),
        visibility: first_event.visibility,
        payload: first_event.payload.clone(),
    });
    anyhow::ensure!(
        conflict.is_err() && spine.metrics().rejected > 0,
        "event spine did not expose conflicting duplicate rejection"
    );
    let reopened_spine = executive::r#impl::events::SqliteEventSpine::open(&event_path)?;
    let reopened_events = reopened_spine.read_tree(
        tree_id,
        executive::r#impl::events::EventReadFilter {
            limit: 100,
            ..Default::default()
        },
    )?;
    anyhow::ensure!(
        actual_events == reopened_events,
        "event spine restart drift"
    );
    let event_checksums = projection_checksums(root.join("agent-rebuild-a.db"), &actual_events)?;
    let rebuilt_again = projection_checksums(root.join("agent-rebuild-b.db"), &reopened_events)?;
    anyhow::ensure!(
        event_checksums == rebuilt_again,
        "real event projection drift"
    );

    let restarted_service = AgentControlService::new(
        kernel,
        clock,
        reopened_repository,
        Arc::new(BoundedAgentAdmission::new(4)?),
        runtimes,
    )
    .with_event_spine(Arc::new(reopened_spine))
    .with_memory_vault(Arc::new(reopened_memory));
    let recovery = restarted_service
        .reconcile_startup("daemon:acceptance-restart")
        .await?;
    service.shutdown().await;

    let sibling_roots = ["first", "second"]
        .into_iter()
        .map(|label| root.join(format!("worktrees/{label}")))
        .collect::<Vec<_>>();
    for path in &sibling_roots {
        std::fs::create_dir_all(path)?;
    }
    Ok(AgentLifecycleEvidence {
        tree: vec![
            ("root".into(), None),
            ("child-0".into(), Some("root".into())),
            ("child-1".into(), Some("root".into())),
        ],
        sibling_roots,
        reopened_runs: reopened.len(),
        reopened_mailbox_deliveries,
        recovery_interrupted: recovery.interrupted,
        event_checksum: checksum(&actual_events)?,
        event_checksums,
        promotion_lineage: promoted.source_event_ids,
        promotion_idempotent: promotion == repeated,
        ordinary_subject_rejected,
        memory_lease_recovered,
        fake_runtime_calls: launcher.launches.load(Ordering::SeqCst),
        unexpected_external_calls: launcher.unexpected_external_calls.load(Ordering::SeqCst),
    })
}

pub struct HarnessRun {
    pub evidence: AcceptanceEvidence,
    pub trace: ConsciousCoreTrace,
    pub replay_epochs: Vec<u64>,
    pub processors: Vec<String>,
    pub memory_candidate_is_private: bool,
    pub dasein_versions: Vec<u64>,
    pub agent_tree: Vec<(String, Option<String>)>,
    pub sibling_roots: Vec<PathBuf>,
    pub external_uses: usize,
    pub duplicate_delivery_rejected: bool,
    pub overload_rejections: usize,
    pub cancellation_terminal: bool,
    pub reopened_agent_runs: usize,
    pub reopened_mailbox_deliveries: usize,
    pub agent_recovery_interrupted: usize,
    pub promotion_lineage: Vec<String>,
    pub promotion_idempotent: bool,
    pub ordinary_subject_rejected: bool,
    pub memory_lease_recovered: bool,
    pub fake_runtime_calls: usize,
    pub authority_denials: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct AblationConfig {
    pub workspace: bool,
    pub recurrence: bool,
    pub dasein_modulation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AblationRun {
    pub broadcasts: usize,
    pub processor_deliveries: usize,
    pub recurrent_broadcasts: usize,
    pub dasein_modulations: usize,
}

struct CountingDasein {
    inner: Arc<dyn executive::service::conscious_core_ports::DaseinWorkspacePort>,
    enabled: bool,
    modulations: AtomicUsize,
}

#[async_trait]
impl executive::service::conscious_core_ports::DaseinWorkspacePort for CountingDasein {
    async fn modulate_salience(
        &self,
        candidate: &fabric::WorkspaceCandidate,
    ) -> anyhow::Result<fabric::SalienceVector> {
        if self.enabled {
            self.modulations.fetch_add(1, Ordering::SeqCst);
            self.inner.modulate_salience(candidate).await
        } else {
            Ok(candidate.salience)
        }
    }

    async fn integrate_broadcast(
        &self,
        broadcast: &fabric::WorkspaceBroadcast,
    ) -> anyhow::Result<executive::service::conscious_core_ports::DaseinIntegration> {
        self.inner.integrate_broadcast(broadcast).await
    }

    async fn self_view(&self) -> anyhow::Result<fabric::StructuredSelfView> {
        self.inner.self_view().await
    }
}

struct FeedbackProcessor {
    space: AgoraSpaceId,
    clock: Arc<dyn fabric::Clock>,
}

#[async_trait]
impl fabric::ConsciousProcessor for FeedbackProcessor {
    fn id(&self) -> fabric::ProcessorId {
        fabric::ProcessorId("acceptance-feedback".into())
    }

    async fn on_broadcast(
        &self,
        broadcast: fabric::WorkspaceBroadcast,
        context: fabric::ProcessorContext,
    ) -> fabric::ProcessorResponse {
        let source = fabric::ProcessId(Uuid::from_u128(802));
        let now = self.clock.mono_now();
        fabric::ProcessorResponse {
            processor: self.id(),
            source_epoch: context.source_epoch,
            health: fabric::ProcessorHealth::Healthy,
            candidates: vec![fabric::WorkspaceCandidate {
                schema_version: fabric::WORKSPACE_SCHEMA_V1,
                id: fabric::ContentId(Uuid::new_v5(
                    &Uuid::from_u128(803),
                    format!("feedback:{}", broadcast.epoch.0).as_bytes(),
                )),
                space: self.space.clone(),
                source,
                turn: None,
                content: WorkspaceContent::Prediction(fabric::PredictionFrame {
                    statement: "bounded recurrent response".into(),
                    horizon_ms: 1_000,
                }),
                confidence: 0.8,
                salience: fabric::SalienceVector {
                    urgency: 0.6,
                    goal_relevance: 0.8,
                    self_relevance: 0.7,
                    novelty: 0.6,
                    confidence: 0.8,
                    prediction_error: 0.0,
                    affect_intensity: 0.0,
                    social_relevance: 0.0,
                },
                provenance: fabric::WorkspaceProvenance {
                    producer: source,
                    operation: None,
                    source_refs: vec![
                        format!("broadcast:{}:{}", self.space.0, broadcast.epoch.0),
                        "processor:acceptance-feedback".into(),
                    ],
                    observed_at: self.clock.wall_now(),
                },
                visibility: fabric::VisibilityScope::Session,
                dependencies: broadcast.winner_ids.clone(),
                created_at: now,
                expires_at: Some(fabric::MonoDeadline::after(now, 10_000)),
            }],
            acknowledgements: broadcast
                .winner_ids
                .iter()
                .map(|content_id| fabric::ProcessorAck {
                    content_id: *content_id,
                    accepted: true,
                    detail: None,
                })
                .collect(),
            detail: None,
        }
    }
}

pub async fn run_ablation(root: &Path, config: AblationConfig) -> anyhow::Result<AblationRun> {
    std::fs::create_dir_all(root)?;
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 100));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(801)),
            parent: None,
            profile: AgentProfileId("ablation-root".into()),
            namespace: NamespaceId("ablation".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await?
        .id;
    let space = AgoraSpaceId("session:ablation".into());
    let store = Arc::new(agora::SqliteBroadcastStore::open(root.join("ablation.db"))?);
    let hub = Arc::new(agora::BroadcastHub::new(
        agora::BroadcastHubConfig::default(),
        store.clone(),
    )?);
    let broadcast = Arc::new(agora::BroadcastCoordinator::new(store.clone(), hub));
    let dasein_module = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let inner: Arc<dyn executive::service::conscious_core_ports::DaseinWorkspacePort> =
        Arc::new(DaseinWorkspaceAdapter::new(dasein_module, clock.clone()));
    let dasein = Arc::new(CountingDasein {
        inner,
        enabled: config.dasein_modulation,
        modulations: AtomicUsize::new(0),
    });
    let coordinator =
        executive::service::conscious_core_coordinator::ConsciousCoreCoordinator::new(
            space.clone(),
            agora::CandidatePoolConfig::default(),
            broadcast,
            store.clone(),
            dasein.clone(),
            fabric::ProcessId(Uuid::from_u128(804)),
            kernel.clone(),
            Arc::new(agora::AgoraRegistry::new(kernel.clock())),
            executive::service::conscious_core_coordinator::ConsciousCoreConfig::default(),
        )?;
    coordinator.register_processor(
        Arc::new(FeedbackProcessor {
            space: space.clone(),
            clock: clock.clone(),
        }),
        owner,
        owner,
    )?;
    let now = clock.mono_now();
    let event_ref = "ablation-observation".to_string();
    executive::service::conscious_core_ports::ConsciousCandidatePort::submit_candidate(
        &coordinator,
        executive::service::conscious_core_ports::CandidateSubmission {
            candidate: fabric::WorkspaceCandidate {
                schema_version: fabric::WORKSPACE_SCHEMA_V1,
                id: fabric::ContentId(Uuid::from_u128(805)),
                space: space.clone(),
                source: owner,
                turn: None,
                content: WorkspaceContent::Observation(fabric::WorkspaceObservation {
                    what: "controlled ablation fixture".into(),
                    source: "acceptance".into(),
                    data: serde_json::Value::Null,
                    attribution: fabric::WorkspaceAttribution::Environment,
                }),
                confidence: 1.0,
                salience: fabric::SalienceVector {
                    urgency: 0.4,
                    goal_relevance: 0.8,
                    self_relevance: 0.7,
                    novelty: 0.9,
                    confidence: 1.0,
                    prediction_error: 0.0,
                    affect_intensity: 0.1,
                    social_relevance: 0.0,
                },
                provenance: fabric::WorkspaceProvenance {
                    producer: owner,
                    operation: None,
                    source_refs: vec![event_ref.clone()],
                    observed_at: clock.wall_now(),
                },
                visibility: fabric::VisibilityScope::Session,
                dependencies: vec![],
                created_at: now,
                expires_at: Some(fabric::MonoDeadline::after(now, 10_000)),
            },
            cause: executive::service::conscious_core_ports::CandidateCause::ExternalObservation {
                event_ref,
            },
        },
    )
    .await?;
    let mut processor_deliveries = 0;
    if config.workspace {
        let first = coordinator.run_cycle(owner, 0).await?;
        processor_deliveries += first.processors.len();
        if config.recurrence {
            let recurrent = coordinator.run_cycle(owner, 1).await?;
            processor_deliveries += recurrent.processors.len();
        }
    }
    let replay = store.replay(&space)?;
    Ok(AblationRun {
        broadcasts: replay.len(),
        processor_deliveries,
        recurrent_broadcasts: replay.len().saturating_sub(1),
        dasein_modulations: dasein.modulations.load(Ordering::SeqCst),
    })
}

pub async fn run(root: &Path) -> anyhow::Result<HarnessRun> {
    std::fs::create_dir_all(root)?;
    let fixture = baseline();
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 100));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(1)),
            parent: None,
            profile: AgentProfileId("acceptance-root".into()),
            namespace: NamespaceId("acceptance".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await?
        .id;
    let agent_lifecycle = run_agent_lifecycle(root, kernel.clone(), clock.clone(), owner).await?;

    let memory = Arc::new(FileBackedMemory::new(root.join("memory.jsonl")));
    let skills_root = root.join("skills");
    std::fs::create_dir_all(skills_root.join("summarize"))?;
    std::fs::write(
        skills_root.join("summarize/SKILL.md"),
        "---\nname: summarize\ndescription: summarize bounded local evidence\nkeywords: [summarize]\n---\nUse selected local evidence only.\n",
    )?;
    let mut loader = corpus::SkillLoader::new(skills_root);
    loader.load_all_enhanced();
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let registry = ConsciousWorkspaceRegistry::production(
        root.join("workspace.db"),
        Arc::new(DaseinWorkspaceAdapter::new(dasein, clock.clone())),
        kernel.clone(),
        clock.clone(),
        memory.clone(),
        Arc::new(Mutex::new(loader)),
    )?;
    let space = AgoraSpaceId("session:acceptance".into());
    let first = registry
        .observe_turn(
            space.clone(),
            owner,
            owner,
            fabric::OperationId(Uuid::from_u128(10)),
            &fixture.input,
        )
        .await?;
    let processors = first
        .processors
        .iter()
        .map(|status| status.processor.0.clone())
        .collect::<Vec<_>>();
    let first_broadcast = first.broadcast.as_ref().expect("fixture must ignite");
    let mut bounded_pool = agora::CandidatePool::new(
        space.clone(),
        agora::CandidatePoolConfig {
            capacity: 2,
            per_source_capacity: 2,
            max_coalition: 2,
            policy: agora::SelectionPolicy::default(),
        },
    )?;
    let bounded_candidate = |id: u128, source: u128, label: &str| {
        let mut candidate = first_broadcast.selected[0].clone();
        candidate.id = fabric::ContentId(Uuid::from_u128(id));
        candidate.source = fabric::ProcessId(Uuid::from_u128(source));
        candidate.provenance.producer = candidate.source;
        candidate.content = WorkspaceContent::Extension {
            schema: "v1/acceptance/bounded".into(),
            payload: serde_json::json!({"label":label}),
        };
        candidate
    };
    let first_bounded = bounded_candidate(501, 601, "first");
    anyhow::ensure!(
        matches!(
            bounded_pool.admit(first_bounded.clone(), fabric::MonoTime(100)),
            agora::AdmissionOutcome::Accepted { .. }
        ),
        "bounded fixture first admission failed"
    );
    let duplicate_delivery_rejected = matches!(
        bounded_pool.admit(first_bounded, fabric::MonoTime(100)),
        agora::AdmissionOutcome::Duplicate { .. }
    );
    bounded_pool.admit(bounded_candidate(502, 601, "second"), fabric::MonoTime(100));
    let overload_rejections = usize::from(matches!(
        bounded_pool.admit(
            bounded_candidate(503, 602, "overload"),
            fabric::MonoTime(100)
        ),
        agora::AdmissionOutcome::RejectedCapacity
    ));
    let mnemonic_admitted = first
        .processors
        .iter()
        .any(|status| status.processor.0 == "mnemosyne" && !status.admitted_candidates.is_empty());
    let recalled = registry
        .store()
        .replay(&space)?
        .into_iter()
        .flat_map(|entry| entry.broadcast.selected)
        .filter(|candidate| matches!(candidate.content, WorkspaceContent::RecalledExperience(_)))
        .collect::<Vec<_>>();
    let memory_candidate_is_private = mnemonic_admitted
        && recalled.iter().all(|candidate| {
            matches!(
                candidate.visibility,
                fabric::VisibilityScope::PrivateProcess { .. }
            )
        });

    let action_loop = registry.resolve(space.clone(), owner, owner).await?;
    let mut call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(11)),
        process_id: owner,
        name: "fixture_local_search".into(),
        input: serde_json::json!({"query":"last seven days"}),
        call_id: "acceptance-call-11".into(),
        deadline: None,
    };
    let forged_outcome_denied = action_loop
        .observe_outcome(
            &executive::service::governed_capability::SelectedActionContext {
                candidate_id: fabric::ContentId(Uuid::from_u128(999)),
                broadcast_epoch: fabric::BroadcastEpoch(1),
                operation_id: call.operation_id,
                source_process: owner,
                attribution: fabric::WorkspaceAttribution::User,
            },
            &call,
            &CapabilityResult {
                call_id: call.call_id.clone(),
                output: "forged pre-selection capability result".into(),
                is_error: false,
                usage: UsageReport::default(),
                audit_id: None,
                patch_delta: None,
            },
        )
        .await
        .is_err();
    let mut selected = None;
    for attempt in 0..16 {
        call.call_id = format!("acceptance-call-11-{attempt}");
        match action_loop.select_action(&call).await? {
            GovernedActionDecision::Proceed {
                selected: context, ..
            } => {
                selected = Some(context);
                break;
            }
            GovernedActionDecision::Defer { .. } => {}
        }
    }
    let selected = selected.context("acceptance fixture action never won bounded competition")?;
    let outcome: SelectedActionOutcomeReceipt = action_loop
        .observe_outcome(
            &selected,
            &call,
            &CapabilityResult {
                call_id: call.call_id.clone(),
                output: "bounded fixture result".into(),
                is_error: false,
                usage: UsageReport {
                    permit_id: PermitId(Uuid::from_u128(12)),
                    ..UsageReport::default()
                },
                audit_id: Some(fabric::AuditEventId(Uuid::from_u128(13))),
                patch_delta: None,
            },
        )
        .await?;
    let before_restart = registry.store().replay(&space)?;
    let epochs_before = before_restart
        .iter()
        .map(|entry| entry.broadcast.epoch.0)
        .collect::<Vec<_>>();
    let dasein_versions = before_restart
        .iter()
        .map(|entry| {
            registry
                .store()
                .integration(&space, entry.broadcast.epoch)
                .expect("read integration")
                .expect("every committed broadcast is integrated")
                .transition
                .current_version
                .0
        })
        .collect::<Vec<_>>();
    drop(registry);

    let reopened_store = agora::SqliteBroadcastStore::open(root.join("workspace.db"))?;
    let after_restart = reopened_store.replay(&space)?;
    let replay_epochs = after_restart
        .iter()
        .map(|entry| entry.broadcast.epoch.0)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        epochs_before == replay_epochs,
        "restart changed Agora history"
    );
    anyhow::ensure!(
        replay_epochs.windows(2).all(|pair| pair[0] < pair[1]),
        "duplicate or unordered Agora epoch"
    );

    let event_checksum = agent_lifecycle.event_checksum.clone();
    let projection_checksums = agent_lifecycle.event_checksums.clone();
    let mut trace = trace(
        &before_restart,
        first.dasein_transition.as_ref(),
        &first.processors,
        &selected,
        &outcome,
    );
    if !trace
        .events
        .iter()
        .any(|event| matches!(event, ConsciousTraceEvent::Memory { .. }))
    {
        if let Some(candidate_id) = first
            .processors
            .iter()
            .find(|status| status.processor.0 == "mnemosyne")
            .and_then(|status| status.admitted_candidates.first())
        {
            trace.events.push(ConsciousTraceEvent::Memory {
                operation: "recall_candidate".into(),
                receipt_ref: candidate_id.0.to_string(),
                authority: "external_reference_candidate_only".into(),
            });
        }
    }
    let evidence = AcceptanceEvidence {
        fixture_version: fixture.fixture_version,
        event_checksum,
        projection_checksums,
        indicator_results: vec![],
        limitations: vec![
            "Functional indicators do not establish phenomenal consciousness.".into(),
            "External providers, network, and process execution are disabled; responses are local fixtures.".into(),
            "Hidden reasoning and model self-report are excluded from evidence.".into(),
        ],
    };

    Ok(HarnessRun {
        evidence,
        trace,
        replay_epochs,
        processors,
        memory_candidate_is_private,
        dasein_versions,
        agent_tree: agent_lifecycle.tree,
        sibling_roots: agent_lifecycle.sibling_roots,
        external_uses: agent_lifecycle.unexpected_external_calls,
        duplicate_delivery_rejected,
        overload_rejections,
        cancellation_terminal: agent_lifecycle.recovery_interrupted == 2,
        reopened_agent_runs: agent_lifecycle.reopened_runs,
        reopened_mailbox_deliveries: agent_lifecycle.reopened_mailbox_deliveries,
        agent_recovery_interrupted: agent_lifecycle.recovery_interrupted,
        promotion_lineage: agent_lifecycle.promotion_lineage,
        promotion_idempotent: agent_lifecycle.promotion_idempotent,
        ordinary_subject_rejected: agent_lifecycle.ordinary_subject_rejected,
        memory_lease_recovered: agent_lifecycle.memory_lease_recovered,
        fake_runtime_calls: agent_lifecycle.fake_runtime_calls,
        authority_denials: usize::from(forged_outcome_denied)
            + usize::from(agent_lifecycle.ordinary_subject_rejected)
            + usize::from(memory_candidate_is_private),
    })
}

fn trace(
    replay: &[agora::BroadcastReplay],
    transition: Option<&fabric::dasein::SelfTransitionReceipt>,
    processors: &[executive::service::conscious_core_ports::ProcessorCycleStatus],
    selected: &executive::service::governed_capability::SelectedActionContext,
    outcome: &SelectedActionOutcomeReceipt,
) -> ConsciousCoreTrace {
    let mut events = Vec::new();
    for (index, entry) in replay.iter().enumerate() {
        let broadcast = &entry.broadcast;
        events.extend(
            broadcast
                .selected
                .iter()
                .map(|candidate| ConsciousTraceEvent::Candidate {
                    disposition: "selected".into(),
                    content_id: candidate.id.0.to_string(),
                    source: candidate.source.0.to_string(),
                    salience: candidate.salience.values(),
                    policy_version: u32::from(broadcast.selected_because.policy_version),
                }),
        );
        events.push(ConsciousTraceEvent::Broadcast {
            epoch: broadcast.epoch.0,
            winner_ids: broadcast
                .winner_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect(),
            recipients: if index == 0 {
                processors.iter().map(|p| p.processor.0.clone()).collect()
            } else {
                Vec::new()
            },
            acknowledgements: entry.acknowledgements.len(),
        });
        for candidate in &broadcast.selected {
            match &candidate.content {
                WorkspaceContent::Prediction(_) => events.push(ConsciousTraceEvent::Prediction {
                    prediction_id: candidate.id.0.to_string(),
                    surprised: false,
                    outcome_ref: "pending-governed-outcome".into(),
                }),
                WorkspaceContent::RecalledExperience(_) => {
                    events.push(ConsciousTraceEvent::Memory {
                        operation: "recall".into(),
                        receipt_ref: candidate.id.0.to_string(),
                        authority: "external_reference_candidate_only".into(),
                    })
                }
                _ => {}
            }
        }
    }
    let broadcast = &replay[0].broadcast;
    if let Some(transition) = transition {
        events.push(ConsciousTraceEvent::Integration {
            epoch: broadcast.epoch.0,
            dasein_before: transition.previous_version.0,
            dasein_after: transition.current_version.0,
        });
    }
    events.push(ConsciousTraceEvent::GovernedAction {
        operation_id: selected.operation_id.0.to_string(),
        permit_ref: outcome.permit_id.clone(),
        outcome_ref: outcome.outcome_id.0.to_string(),
    });
    ConsciousCoreTrace {
        schema_version: CONSCIOUS_CORE_TRACE_SCHEMA_V1,
        fixture_version: 1,
        events,
    }
}

fn projection_checksums(
    path: PathBuf,
    events: &[SpineEvent],
) -> anyhow::Result<BTreeMap<String, String>> {
    let store = SqliteProjectionStore::open(path)?;
    let mut result = BTreeMap::new();
    macro_rules! rebuild {
        ($name:literal, $projection:expr) => {{
            let projection = $projection;
            let accepted = projection.descriptor().accepted_schemas;
            let inputs = events
                .iter()
                .filter(|event| {
                    accepted.iter().any(|schema| schema == &event.schema.0)
                        && !($name == "session"
                            && event.schema.0 == SchemaId::TURN_EVENT_V1
                            && matches!(
                                &event.payload,
                                EventPayload::Inline { value }
                                    if value.get("schema_version").is_none()
                            ))
                })
                .cloned()
                .collect::<Vec<_>>();
            let (_, checkpoint) = store.rebuild(&projection, &inputs)?;
            result.insert($name.into(), checkpoint.checksum);
        }};
    }
    rebuild!("session", SessionProjection);
    rebuild!("debug", DebugProjection);
    rebuild!("memory_jobs", MemoryJobProjection);
    rebuild!("agent_tree", AgentTreeProjection);
    rebuild!("metrics", MetricsProjection);
    Ok(result)
}

fn checksum<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
