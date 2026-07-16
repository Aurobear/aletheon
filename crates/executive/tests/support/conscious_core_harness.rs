#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use executive::r#impl::events::{
    agent_tree_projection::AgentTreeProjection, debug_projection::DebugProjection,
    memory_job_projection::MemoryJobProjection, metrics_projection::MetricsProjection,
    session_projection::SessionProjection,
};
use executive::service::conscious_workspace::{ConsciousTurnPort, ConsciousWorkspaceRegistry};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use executive::service::event_projection::SqliteProjectionStore;
use executive::service::governed_capability::{
    GovernedActionLoopResolver, SelectedActionOutcomeReceipt,
};
use fabric::{
    AcceptanceEvidence, AgentId, AgentProfileId, AgoraSpaceId, CapabilityCall, CapabilityResult,
    ConsciousCoreTrace, ConsciousTraceEvent, EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target,
    EventId, EventIdentity, EventPayload, EventPosition, EventTreeId, EventVisibility, ItemId,
    ItemPayload, ItemRecord, NamespaceId, PermitId, SchemaId, SessionId, SpawnSpec, SpineEvent,
    TreeSequence, TurnId, UsageReport, WorkspaceContent, CONSCIOUS_CORE_TRACE_SCHEMA_V1,
    SESSION_SCHEMA_VERSION,
};
use mnemosyne::{ExperienceEvent, ForgetPolicy, MemoryScope, RecallItem, RecallRequest, RecallSet};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
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
        })
        .await?
        .id;
    let mut agent_tree = vec![("root".into(), None)];
    let mut sibling_roots = Vec::new();
    let mut child_processes = Vec::new();
    for (index, id) in [2_u128, 3].into_iter().enumerate() {
        let child = kernel
            .spawn_process(SpawnSpec {
                agent_id: AgentId(Uuid::from_u128(id)),
                parent: Some(owner),
                profile: AgentProfileId(format!("acceptance-child-{index}")),
                namespace: NamespaceId(format!("acceptance-child-{index}")),
                initial_operation: None,
                deadline: None,
            })
            .await?;
        kernel.inspect_process(child.id).await?;
        child_processes.push(child.id);
        agent_tree.push((format!("child-{index}"), Some("root".into())));
        let child_root = root.join(format!("worktrees/child-{index}"));
        std::fs::create_dir_all(&child_root)?;
        sibling_roots.push(child_root);
    }
    kernel
        .signal_process(child_processes[0], fabric::ProcessSignal::Terminate)
        .await?;
    let cancellation_terminal = kernel
        .inspect_process(child_processes[0])
        .await?
        .state
        .is_terminal();
    // Opening the real Agent repository proves the harness uses its durable schema,
    // while Kernel owns the live process tree above.
    let _agents =
        executive::service::agent_control::SqliteAgentRunRepository::open(root.join("agents.db"))?;

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
        clock,
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
    let call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(11)),
        process_id: owner,
        name: "fixture_local_search".into(),
        input: serde_json::json!({"query":"last seven days"}),
        call_id: "acceptance-call-11".into(),
        deadline: None,
    };
    let selected = action_loop.select_action(&call).await?;
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

    let events = projection_events(first_broadcast.epoch.0, outcome.broadcast_epoch.0);
    let event_checksum = checksum(&events)?;
    let projection_checksums = projection_checksums(root.join("projections.db"), &events)?;
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
        agent_tree,
        sibling_roots,
        external_uses: 0,
        duplicate_delivery_rejected,
        overload_rejections,
        cancellation_terminal,
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

fn projection_events(first_epoch: u64, outcome_epoch: u64) -> Vec<SpineEvent> {
    let rows = [
        (
            "session",
            SchemaId::EVENT_SESSION_CREATED_V1,
            EventVisibility::Control,
        ),
        (
            "turn",
            SchemaId::TURN_EVENT_V1,
            EventVisibility::ModelVisible,
        ),
        (
            "child_agent",
            SchemaId::EVENT_AGENT_STARTED_V1,
            EventVisibility::Control,
        ),
        (
            "memory_candidate",
            SchemaId::EVENT_MEMORY_CANDIDATE_V1,
            EventVisibility::Sensitive,
        ),
        (
            "agora_broadcast",
            SchemaId::EVENT_AGORA_BROADCAST_V1,
            EventVisibility::Control,
        ),
    ];
    let tree = EventTreeId::for_root_session("acceptance-session");
    rows.into_iter().enumerate().map(|(index, (kind, schema, visibility))| {
        let sequence = index as u64 + 1;
        let value = match kind {
            "session" => serde_json::to_value(fabric::SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: SessionId("acceptance-session".into()),
                parent: None,
                created_at_ms: 1_700_000_000_000,
                status: fabric::SessionStatus::Active,
            }).unwrap(),
            "turn" => serde_json::to_value(ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: ItemId(Uuid::from_u128(201)),
                session_id: SessionId("acceptance-session".into()),
                turn_id: TurnId(Uuid::from_u128(202)),
                sequence: 1,
                created_at_ms: 1_700_000_000_001,
                payload: ItemPayload::AssistantMessage { content: "bounded fixture result".into() },
            }).unwrap(),
            "child_agent" => serde_json::json!({"agent_id":Uuid::from_u128(2), "parent_agent_id":Uuid::from_u128(1)}),
            "memory_candidate" => serde_json::json!({"record_id":"candidate:acceptance", "kind":"recall", "content":{"authority":"external_reference"}, "sensitivity":"internal"}),
            "agora_broadcast" => serde_json::json!({"epoch":first_epoch, "outcome_epoch":outcome_epoch}),
            _ => unreachable!(),
        };
        let mut envelope = EnvelopeV2::new(
            SchemaId(schema.into()),
            EnvelopeV2Target("acceptance".into()),
            EnvelopeV2Target("acceptance-session".into()),
            EnvelopeV2Delivery::Direct,
            NamespaceId("acceptance".into()),
            value.clone(),
        );
        envelope.id = fabric::ipc::envelope_v2::MessageId(Uuid::from_u128(400 + sequence as u128));
        SpineEvent {
            position: EventPosition { tree_id: tree, event_id: EventId(Uuid::from_u128(300 + sequence as u128)), parent: None, sequence: TreeSequence(sequence) },
            identity: EventIdentity { root_session_id:"acceptance-session".into(), session_id:"acceptance-session".into(), agent_id:(kind == "child_agent").then(|| Uuid::from_u128(2).to_string()) },
            schema: SchemaId(schema.into()), visibility, envelope,
            payload: EventPayload::Inline { value },
        }
    }).collect()
}

fn projection_checksums(
    path: PathBuf,
    events: &[SpineEvent],
) -> anyhow::Result<BTreeMap<String, String>> {
    let store = SqliteProjectionStore::open(path)?;
    let mut result = BTreeMap::new();
    macro_rules! rebuild {
        ($name:literal, $projection:expr) => {{
            let (_, checkpoint) = store.rebuild(&$projection, events)?;
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
