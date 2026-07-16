//! Production registry and domain adapters for recurrent conscious workspaces.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agora::{
    BroadcastCoordinator, BroadcastHub, BroadcastHubConfig, CandidatePoolConfig, SelectionPolicy,
    SqliteBroadcastStore,
};
use aletheon_kernel::KernelRuntime;
use anyhow::Context;
use async_trait::async_trait;
use fabric::{
    AgoraSpaceId, Clock, ConsciousContextProjection, ConsciousProcessor, ContentId, MonoDeadline,
    PredictionFrame, ProcessId, ProcessorAck, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, SalienceVector, VisibilityScope, WorkspaceAttribution, WorkspaceBroadcast,
    WorkspaceCandidate, WorkspaceContent, WorkspaceObservation, WorkspaceProvenance,
    WorkspaceReflection, WORKSPACE_SCHEMA_V1,
};
use mnemosyne::MemoryWorkspaceProjector;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::agent_control::AgentCandidateSubmissionPort;
use super::conscious_action::ConsciousActionBridge;
use super::conscious_core_coordinator::{ConsciousCoreConfig, ConsciousCoreCoordinator};
use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
    ConsciousCycleReceipt, DaseinWorkspacePort, LatestConsciousContextPort,
};
use super::governed_capability::{GovernedActionLoop, GovernedActionLoopResolver};

const WORKSPACE_NAMESPACE: Uuid = Uuid::from_u128(0x4021_c073_f88b_45c9_b913_89b9_42f8_0671);
const PROCESSOR_TTL: Duration = Duration::from_secs(60);

#[async_trait]
pub trait ConsciousTurnPort: GovernedActionLoopResolver {
    async fn observe_turn(
        &self,
        space: AgoraSpaceId,
        owner: ProcessId,
        root: ProcessId,
        operation: fabric::OperationId,
        input: &str,
    ) -> anyhow::Result<ConsciousCycleReceipt>;
}

pub struct ConsciousWorkspaceRegistry {
    store: Arc<SqliteBroadcastStore>,
    broadcast: Arc<BroadcastCoordinator>,
    dasein: Arc<dyn DaseinWorkspacePort>,
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    memory_service: Arc<dyn mnemosyne::MemoryService>,
    skills: Arc<Mutex<corpus::SkillLoader>>,
    pool_config: CandidatePoolConfig,
    core_config: ConsciousCoreConfig,
    spaces: RwLock<HashMap<AgoraSpaceId, Arc<ConsciousCoreCoordinator>>>,
}

impl ConsciousWorkspaceRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        path: impl AsRef<Path>,
        dasein: Arc<dyn DaseinWorkspacePort>,
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        skills: Arc<Mutex<corpus::SkillLoader>>,
        pool_config: CandidatePoolConfig,
        core_config: ConsciousCoreConfig,
    ) -> anyhow::Result<Self> {
        let store = Arc::new(SqliteBroadcastStore::open(path)?);
        let hub = Arc::new(BroadcastHub::new(
            BroadcastHubConfig::default(),
            store.clone(),
        )?);
        let broadcast = Arc::new(BroadcastCoordinator::new(store.clone(), hub));
        Ok(Self {
            store,
            broadcast,
            dasein,
            kernel,
            clock,
            memory_service: memory,
            skills,
            pool_config,
            core_config,
            spaces: RwLock::new(HashMap::new()),
        })
    }

    pub fn production(
        path: impl AsRef<Path>,
        dasein: Arc<dyn DaseinWorkspacePort>,
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        skills: Arc<Mutex<corpus::SkillLoader>>,
    ) -> anyhow::Result<Self> {
        Self::open(
            path,
            dasein,
            kernel,
            clock,
            memory,
            skills,
            CandidatePoolConfig {
                capacity: 256,
                per_source_capacity: 32,
                max_coalition: 8,
                policy: SelectionPolicy::default(),
            },
            ConsciousCoreConfig::default(),
        )
    }

    pub fn store(&self) -> Arc<SqliteBroadcastStore> {
        self.store.clone()
    }

    /// Run a later C01 competition over already-admitted candidates. Agent
    /// projection deliberately never calls this: admission and global
    /// selection remain separate causal steps.
    pub async fn run_pending_cycle(
        &self,
        space: AgoraSpaceId,
        owner: ProcessId,
        root: ProcessId,
        recurrence_depth: u16,
    ) -> anyhow::Result<ConsciousCycleReceipt> {
        self.coordinator(space, owner, root)?
            .run_cycle(owner, recurrence_depth)
            .await
    }

    fn coordinator(
        &self,
        space: AgoraSpaceId,
        recipient: ProcessId,
        root: ProcessId,
    ) -> anyhow::Result<Arc<ConsciousCoreCoordinator>> {
        if let Some(coordinator) = self.spaces.read().get(&space).cloned() {
            return Ok(coordinator);
        }
        let mut spaces = self.spaces.write();
        if let Some(coordinator) = spaces.get(&space).cloned() {
            return Ok(coordinator);
        }
        let dasein_source = processor_source(&space, "dasein-integration");
        let coordinator = Arc::new(ConsciousCoreCoordinator::new(
            space.clone(),
            self.pool_config.clone(),
            self.broadcast.clone(),
            self.store.clone(),
            self.dasein.clone(),
            dasein_source,
            self.kernel.clone(),
            self.core_config.clone(),
        )?);
        for processor in self.processors(&space) {
            coordinator.register_processor(processor, recipient, root)?;
        }
        spaces.insert(space, coordinator.clone());
        Ok(coordinator)
    }

    fn processors(&self, space: &AgoraSpaceId) -> Vec<Arc<dyn ConsciousProcessor>> {
        vec![
            Arc::new(DomainProcessor::new(
                space,
                ProcessorKind::Dasein,
                self.clock.clone(),
            )),
            Arc::new(DomainProcessor::new(
                space,
                ProcessorKind::Cognit,
                self.clock.clone(),
            )),
            Arc::new(DomainProcessor::with_memory(
                space,
                self.clock.clone(),
                self.memory_service.clone(),
            )),
            Arc::new(DomainProcessor::new(
                space,
                ProcessorKind::Metacog,
                self.clock.clone(),
            )),
            Arc::new(DomainProcessor::with_skills(
                space,
                self.clock.clone(),
                self.skills.clone(),
            )),
        ]
    }
}

#[async_trait]
impl AgentCandidateSubmissionPort for ConsciousWorkspaceRegistry {
    async fn submit_agent_candidate(
        &self,
        submission: CandidateSubmission,
        recipient: ProcessId,
        root: ProcessId,
    ) -> anyhow::Result<super::conscious_core_ports::CandidateSubmissionReceipt> {
        let coordinator = self.coordinator(submission.candidate.space.clone(), recipient, root)?;
        coordinator.submit_candidate(submission).await
    }
}

#[async_trait]
impl LatestConsciousContextPort for ConsciousWorkspaceRegistry {
    async fn latest_context(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection> {
        let coordinator = self
            .spaces
            .read()
            .get(space)
            .cloned()
            .context("conscious workspace has not observed a turn")?;
        coordinator.latest_context(space).await
    }
}

#[async_trait]
impl GovernedActionLoopResolver for ConsciousWorkspaceRegistry {
    async fn resolve(
        &self,
        space: AgoraSpaceId,
        source: ProcessId,
        root: ProcessId,
    ) -> anyhow::Result<Arc<dyn GovernedActionLoop>> {
        let coordinator = self.coordinator(space, source, root)?;
        Ok(Arc::new(ConsciousActionBridge::new(
            coordinator,
            source,
            root,
            self.clock.clone(),
            Duration::from_secs(60),
        )?))
    }
}

#[async_trait]
impl ConsciousTurnPort for ConsciousWorkspaceRegistry {
    async fn observe_turn(
        &self,
        space: AgoraSpaceId,
        owner: ProcessId,
        root: ProcessId,
        operation: fabric::OperationId,
        input: &str,
    ) -> anyhow::Result<ConsciousCycleReceipt> {
        let coordinator = self.coordinator(space.clone(), owner, root)?;
        let event_ref = format!("user-turn:{}", operation.0);
        let now = self.clock.mono_now();
        let input = truncate(input, 8 * 1024);
        let candidate = WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(Uuid::new_v5(
                &WORKSPACE_NAMESPACE,
                format!("{}:{event_ref}", space.0).as_bytes(),
            )),
            space,
            source: owner,
            turn: None,
            content: WorkspaceContent::Observation(WorkspaceObservation {
                what: input.clone(),
                source: "user-turn".into(),
                data: serde_json::json!({
                    "input_sha256": format!("{:x}", Sha256::digest(input.as_bytes()))
                }),
                attribution: WorkspaceAttribution::User,
            }),
            confidence: 1.0,
            salience: max_salience(),
            provenance: WorkspaceProvenance {
                producer: owner,
                operation: Some(operation),
                source_refs: vec![event_ref.clone()],
                observed_at: self.clock.wall_now(),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![],
            created_at: now,
            expires_at: Some(MonoDeadline::after(now, 60_000)),
        };
        let observation_id = candidate.id;
        let admission = coordinator
            .submit_candidate(CandidateSubmission {
                candidate,
                cause: CandidateCause::ExternalObservation { event_ref },
            })
            .await?;
        anyhow::ensure!(
            matches!(
                admission.status,
                CandidateAdmissionStatus::Accepted | CandidateAdmissionStatus::Duplicate
            ),
            "turn observation was not admitted: {:?}",
            admission.status
        );
        let receipt = coordinator.run_cycle(owner, 0).await?;
        let broadcast = receipt
            .broadcast
            .as_ref()
            .context("turn observation did not ignite")?;
        anyhow::ensure!(
            broadcast.winner_ids.contains(&observation_id),
            "turn observation was not selected"
        );
        let recurrence = coordinator.run_cycle(owner, 1).await?;
        anyhow::ensure!(
            recurrence.broadcast.is_some(),
            "turn processors produced no recurrent selection"
        );
        Ok(receipt)
    }
}

#[derive(Clone)]
enum ProcessorKind {
    Dasein,
    Cognit,
    Mnemosyne(Arc<dyn mnemosyne::MemoryService>),
    Metacog,
    Corpus(Arc<Mutex<corpus::SkillLoader>>),
}

impl ProcessorKind {
    fn id(&self) -> &'static str {
        match self {
            Self::Dasein => "dasein",
            Self::Cognit => "cognit",
            Self::Mnemosyne(_) => "mnemosyne",
            Self::Metacog => "metacog",
            Self::Corpus(_) => "corpus",
        }
    }
}

struct DomainProcessor {
    id: ProcessorId,
    source: ProcessId,
    kind: ProcessorKind,
    clock: Arc<dyn Clock>,
}

impl DomainProcessor {
    fn new(space: &AgoraSpaceId, kind: ProcessorKind, clock: Arc<dyn Clock>) -> Self {
        let id = ProcessorId(kind.id().into());
        Self {
            source: processor_source(space, &id.0),
            id,
            kind,
            clock,
        }
    }

    fn with_memory(
        space: &AgoraSpaceId,
        clock: Arc<dyn Clock>,
        memory: Arc<dyn mnemosyne::MemoryService>,
    ) -> Self {
        Self::new(space, ProcessorKind::Mnemosyne(memory), clock)
    }

    fn with_skills(
        space: &AgoraSpaceId,
        clock: Arc<dyn Clock>,
        skills: Arc<Mutex<corpus::SkillLoader>>,
    ) -> Self {
        Self::new(space, ProcessorKind::Corpus(skills), clock)
    }

    fn candidate(
        &self,
        broadcast: &WorkspaceBroadcast,
        index: usize,
        content: WorkspaceContent,
        salience: SalienceVector,
    ) -> WorkspaceCandidate {
        let now = self.clock.mono_now();
        WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(Uuid::new_v5(
                &WORKSPACE_NAMESPACE,
                format!(
                    "processor:{}:{}:{}:{}",
                    broadcast.space.0, broadcast.epoch.0, self.id.0, index
                )
                .as_bytes(),
            )),
            space: broadcast.space.clone(),
            source: self.source,
            turn: None,
            content,
            confidence: 0.8,
            salience,
            provenance: WorkspaceProvenance {
                producer: self.source,
                operation: None,
                source_refs: vec![
                    format!("broadcast:{}:{}", broadcast.space.0, broadcast.epoch.0),
                    format!("processor:{}", self.id.0),
                ],
                observed_at: self.clock.wall_now(),
            },
            visibility: VisibilityScope::Session,
            dependencies: broadcast.winner_ids.clone(),
            created_at: now,
            expires_at: Some(MonoDeadline::after(now, PROCESSOR_TTL.as_millis() as u64)),
        }
    }
}

#[async_trait]
impl ConsciousProcessor for DomainProcessor {
    fn id(&self) -> ProcessorId {
        self.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        let query = broadcast_summary(&broadcast);
        let mut health = ProcessorHealth::Healthy;
        let mut detail = None;
        let candidates = match &self.kind {
            ProcessorKind::Dasein => vec![],
            ProcessorKind::Cognit => vec![self.candidate(
                &broadcast,
                0,
                WorkspaceContent::Prediction(PredictionFrame {
                    statement: format!(
                        "next deliberation should resolve: {}",
                        truncate(&query, 512)
                    ),
                    horizon_ms: 30_000,
                }),
                processor_salience(0.6, 0.7, 0.5),
            )],
            ProcessorKind::Mnemosyne(memory) => {
                match memory
                    .recall(mnemosyne::RecallRequest {
                        session: broadcast.space.0.clone(),
                        query: truncate_bytes(&query, mnemosyne::RecallRequest::MAX_QUERY_BYTES),
                        max_items: context.max_candidates.clamp(1, 4),
                        max_content_bytes: 16 * 1024,
                        current_at: Some(fabric::wall_to_datetime(self.clock.wall_now())),
                        include_historical: false,
                    })
                    .await
                {
                    Ok(recall) => match mnemosyne::DefaultMemoryWorkspaceProjector.project(
                        &recall,
                        mnemosyne::MemoryProjectionLimits {
                            max_items: context.max_candidates.clamp(1, 8),
                            ..Default::default()
                        },
                    ) {
                        Ok(projection) => {
                            if !projection.degraded_sources.is_empty() {
                                health = ProcessorHealth::Degraded;
                                detail = Some(format!(
                                    "memory sources degraded: {}",
                                    projection.degraded_sources.join(",")
                                ));
                            }
                            match projection.to_candidates(&mnemosyne::MemoryCandidateContext {
                                space: broadcast.space.clone(),
                                source: self.source,
                                source_epoch: broadcast.epoch,
                                dependencies: broadcast.winner_ids.clone(),
                                created_at: self.clock.mono_now(),
                                ttl_ms: PROCESSOR_TTL.as_millis() as u64,
                            }) {
                                Ok(candidates) => candidates,
                                Err(error) => {
                                    health = ProcessorHealth::Degraded;
                                    detail = Some(format!(
                                        "memory candidate projection failed: {error}"
                                    ));
                                    vec![]
                                }
                            }
                        }
                        Err(error) => {
                            health = ProcessorHealth::Degraded;
                            detail = Some(format!("bounded memory projection failed: {error}"));
                            vec![]
                        }
                    },
                    Err(error) => {
                        health = ProcessorHealth::Degraded;
                        detail = Some(format!("bounded recall failed: {error}"));
                        vec![]
                    }
                }
            }
            ProcessorKind::Metacog => vec![self.candidate(
                &broadcast,
                0,
                WorkspaceContent::Reflection(WorkspaceReflection {
                    findings: vec![format!(
                        "review confidence and conflicts for epoch {}",
                        broadcast.epoch.0
                    )],
                    confidence: 0.7,
                }),
                processor_salience(0.4, 0.5, 0.6),
            )],
            ProcessorKind::Corpus(skills) => {
                let loader = skills.lock().await;
                let keywords = loader
                    .plugins()
                    .iter()
                    .filter(|plugin| !plugin.keywords.is_empty())
                    .map(|plugin| corpus::skill::keyword_matcher::SkillKeywords {
                        name: plugin.name.clone(),
                        keywords: plugin.keywords.clone(),
                        body: plugin.system_prompt.clone(),
                    })
                    .collect::<Vec<_>>();
                let matched = corpus::skill::keyword_matcher::match_skills(&query, &keywords);
                if matched.is_empty() {
                    vec![]
                } else {
                    vec![self.candidate(
                        &broadcast,
                        0,
                        WorkspaceContent::Extension {
                            schema: "v1/corpus/skill-projection".into(),
                            payload: serde_json::json!({
                                "matched": matched.into_iter().take(3).map(|item| truncate(&item, 2048)).collect::<Vec<_>>()
                            }),
                        },
                        processor_salience(0.4, 0.7, 0.6),
                    )]
                }
            }
        };
        ProcessorResponse {
            processor: self.id.clone(),
            source_epoch: context.source_epoch,
            health,
            candidates,
            acknowledgements: broadcast
                .winner_ids
                .iter()
                .map(|content_id| ProcessorAck {
                    content_id: *content_id,
                    accepted: true,
                    detail: None,
                })
                .collect(),
            detail,
        }
    }
}

fn processor_source(space: &AgoraSpaceId, processor: &str) -> ProcessId {
    ProcessId(Uuid::new_v5(
        &WORKSPACE_NAMESPACE,
        format!("{}:{processor}", space.0).as_bytes(),
    ))
}

fn max_salience() -> SalienceVector {
    SalienceVector {
        urgency: 1.0,
        goal_relevance: 1.0,
        self_relevance: 1.0,
        novelty: 1.0,
        confidence: 1.0,
        prediction_error: 1.0,
        affect_intensity: 1.0,
        social_relevance: 1.0,
    }
}

fn processor_salience(urgency: f32, goal: f32, confidence: f32) -> SalienceVector {
    SalienceVector {
        urgency,
        goal_relevance: goal,
        self_relevance: 0.5,
        novelty: 0.5,
        confidence,
        prediction_error: 0.3,
        affect_intensity: 0.2,
        social_relevance: 0.2,
    }
}

fn broadcast_summary(broadcast: &WorkspaceBroadcast) -> String {
    let summary = broadcast
        .selected
        .iter()
        .filter_map(|candidate| serde_json::to_string(&candidate.content).ok())
        .collect::<Vec<_>>()
        .join("\n");
    if summary.trim().is_empty() {
        format!("workspace epoch {}", broadcast.epoch.0)
    } else {
        truncate(&summary, mnemosyne::RecallRequest::MAX_QUERY_BYTES)
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn truncate_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
