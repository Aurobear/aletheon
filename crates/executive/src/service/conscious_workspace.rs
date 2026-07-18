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
    AgoraSpaceId, CapabilityBatchDecision, CapabilityBatchPlan, CapabilityCall, Clock,
    ConsciousArbitrationMode, ConsciousContextProjection, ConsciousFieldReadout,
    ConsciousProcessor, ContentId, FieldDecisionKind, FieldDecisionReason,
    LatestConsciousContextPort, MonoDeadline, PredictionFrame, ProcessId, ProcessorAck,
    ProcessorContext, ProcessorHealth, ProcessorId, ProcessorResponse, SalienceVector,
    VisibilityScope, WorkspaceAttribution, WorkspaceBroadcast, WorkspaceCandidate,
    WorkspaceContent, WorkspaceObservation, WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_DIAGNOSTIC_MODULATIONS: usize = 256;

fn bound_recent_modulations(
    mut events: Vec<fabric::ConsciousTraceEvent>,
    requested_limit: usize,
) -> (Vec<fabric::ConsciousTraceEvent>, usize) {
    let total = events.len();
    let limit = requested_limit.clamp(1, MAX_DIAGNOSTIC_MODULATIONS);
    if events.len() > limit {
        events.drain(..events.len() - limit);
    }
    (events, total)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConsciousFieldDiagnostics {
    pub space: AgoraSpaceId,
    pub indicators: fabric::FieldMetricIndicators,
    pub modulations: Vec<fabric::ConsciousTraceEvent>,
    pub total_modulations: usize,
    pub truncated: bool,
}

use super::agent_control::AgentCandidateSubmissionPort;
use super::conscious_action::ConsciousActionBridge;
use super::conscious_core_coordinator::{
    ConsciousCoreConfig, ConsciousCoreCoordinator, ProcessorRegistration,
};
use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
    ConsciousCycleReceipt, DaseinWorkspacePort,
};
use super::governed_capability::{GovernedActionLoop, GovernedActionLoopResolver};
use crate::r#impl::conscious::{
    AgentAdapter, CorpusProcessor, MetacogProcessor, MnemosyneProcessor,
};

const WORKSPACE_NAMESPACE: Uuid = Uuid::from_u128(0x4021_c073_f88b_45c9_b913_89b9_42f8_0671);
const PROCESSOR_TTL: Duration = Duration::from_secs(60);

/// Sort bounded priorities descending while retaining provider order for ties.
pub fn stable_priority_order(priorities: &[(String, f32)]) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        priorities
            .iter()
            .all(|(_, priority)| priority.is_finite() && (0.0..=1.0).contains(priority)),
        "conscious batch priority is outside [0,1]"
    );
    let mut indices: Vec<usize> = (0..priorities.len()).collect();
    indices.sort_by(|left, right| {
        priorities[*right]
            .1
            .total_cmp(&priorities[*left].1)
            .then_with(|| left.cmp(right))
    });
    Ok(indices
        .into_iter()
        .map(|index| priorities[index].0.clone())
        .collect())
}

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

    async fn batch_planner(
        &self,
        space: AgoraSpaceId,
    ) -> anyhow::Result<Arc<dyn cognit::harness::BatchPlanner>>;
}

struct ConsciousWorkspaceBatchPlanner {
    coordinator: Arc<ConsciousCoreCoordinator>,
    mode: ConsciousArbitrationMode,
}

impl ConsciousWorkspaceBatchPlanner {
    fn identity(&self, calls: &[CapabilityCall]) -> CapabilityBatchPlan {
        let mut plan = CapabilityBatchPlan::identity(calls);
        plan.mode = self.mode;
        plan
    }

    fn priority(
        readout: &ConsciousFieldReadout,
        projection: &ConsciousContextProjection,
        call: &CapabilityCall,
    ) -> f32 {
        projection
            .latest_broadcast
            .as_ref()
            .and_then(|broadcast| {
                broadcast.selected.iter().find_map(|candidate| {
                    matches!(
                        &candidate.content,
                        WorkspaceContent::ActionProposal(action) if action.id == call.call_id
                    )
                    .then_some((candidate.confidence * readout.precision).clamp(0.0, 1.0))
                })
            })
            .unwrap_or(readout.precision)
    }

    fn metric_ref(&self, epoch: fabric::BroadcastEpoch) -> String {
        self.coordinator
            .field_metric_snapshots()
            .into_iter()
            .rev()
            .find(|snapshot| snapshot.broadcast_epoch == epoch.0)
            .map(|snapshot| snapshot.trace_event_id)
            .filter(|reference| !reference.trim().is_empty())
            .unwrap_or_else(|| format!("broadcast:{}:{}", self.coordinator.space().0, epoch.0))
    }

    fn record_reorders(
        &self,
        calls: &[CapabilityCall],
        plan: &CapabilityBatchPlan,
        epoch: fabric::BroadcastEpoch,
    ) -> anyhow::Result<()> {
        let metric_ref = self.metric_ref(epoch);
        for decision in plan
            .decisions
            .iter()
            .filter(|decision| decision.decision == FieldDecisionKind::Reorder)
        {
            let call = calls
                .iter()
                .find(|call| call.call_id == decision.call_id)
                .context("reorder decision references an unknown capability call")?;
            let event = reorder_trace_event(self.mode, call, decision, epoch, &metric_ref);
            self.coordinator.record_field_modulation(&event)?;
        }
        Ok(())
    }
}

fn reorder_trace_event(
    mode: ConsciousArbitrationMode,
    call: &CapabilityCall,
    decision: &CapabilityBatchDecision,
    epoch: fabric::BroadcastEpoch,
    metric_ref: &str,
) -> fabric::ConsciousTraceEvent {
    fabric::ConsciousTraceEvent::FieldModulation {
        mode,
        decision: FieldDecisionKind::Reorder,
        reason: FieldDecisionReason::Selected,
        operation_id: call.operation_id.0.to_string(),
        call_id: call.call_id.clone(),
        broadcast_epoch: Some(epoch.0),
        baseline: None,
        effective: Some(f64::from(decision.priority)),
        delta: None,
        metric_ref: metric_ref.to_owned(),
    }
}

#[async_trait]
impl cognit::harness::BatchPlanner for ConsciousWorkspaceBatchPlanner {
    async fn plan(&self, calls: Vec<CapabilityCall>) -> anyhow::Result<CapabilityBatchPlan> {
        let projection = match self
            .coordinator
            .latest_context(self.coordinator.space())
            .await
        {
            Ok(projection) => projection,
            Err(error) => {
                tracing::warn!(%error, "conscious batch projection unavailable; preserving provider order");
                return Ok(self.identity(&calls));
            }
        };
        let readout = match ConsciousFieldReadout::from_projection(&projection) {
            Ok(Some(readout)) => readout,
            Ok(None) => return Ok(self.identity(&calls)),
            Err(error) => {
                tracing::warn!(%error, "conscious batch projection invalid; preserving provider order");
                return Ok(self.identity(&calls));
            }
        };
        let priorities: Vec<f32> = calls
            .iter()
            .map(|call| Self::priority(&readout, &projection, call))
            .collect();
        let ordered_call_ids = stable_priority_order(
            &calls
                .iter()
                .zip(priorities.iter())
                .map(|(call, priority)| (call.call_id.clone(), *priority))
                .collect::<Vec<_>>(),
        )?;
        let decisions = calls
            .iter()
            .enumerate()
            .map(|(original_index, call)| CapabilityBatchDecision {
                call_id: call.call_id.clone(),
                decision: if ordered_call_ids
                    .iter()
                    .position(|call_id| call_id == &call.call_id)
                    == Some(original_index)
                {
                    FieldDecisionKind::Proceed
                } else {
                    FieldDecisionKind::Reorder
                },
                reason: FieldDecisionReason::Selected,
                priority: priorities[original_index],
                broadcast_epoch: Some(readout.epoch),
            })
            .collect();
        let plan = CapabilityBatchPlan {
            mode: self.mode,
            ordered_call_ids,
            decisions,
        };
        plan.validate_against(&calls)?;
        if let Err(error) = self.record_reorders(&calls, &plan, readout.epoch) {
            match self.mode {
                ConsciousArbitrationMode::Observe => tracing::warn!(
                    %error,
                    "conscious batch reorder trace unavailable; preserving observable execution"
                ),
                ConsciousArbitrationMode::Enforce => {
                    tracing::warn!(
                        %error,
                        "conscious batch reorder trace unavailable; suppressing untraced reorder"
                    );
                    return Ok(self.identity(&calls));
                }
            }
        }
        Ok(plan)
    }
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
        Self::production_with_mode(
            path,
            dasein,
            kernel,
            clock,
            memory,
            skills,
            ConsciousArbitrationMode::Observe,
        )
    }

    pub fn production_with_mode(
        path: impl AsRef<Path>,
        dasein: Arc<dyn DaseinWorkspacePort>,
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        skills: Arc<Mutex<corpus::SkillLoader>>,
        arbitration_mode: ConsciousArbitrationMode,
    ) -> anyhow::Result<Self> {
        let core_config = ConsciousCoreConfig {
            arbitration_mode,
            ..ConsciousCoreConfig::default()
        };
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
            core_config,
        )
    }

    pub fn store(&self) -> Arc<SqliteBroadcastStore> {
        self.store.clone()
    }

    /// Read bounded, content-free field metrics plus durable modulation traces.
    /// Diagnostics never creates a workspace or advances its recurrence cycle.
    pub fn field_diagnostics(
        &self,
        space: &AgoraSpaceId,
        requested_limit: usize,
    ) -> anyhow::Result<Option<ConsciousFieldDiagnostics>> {
        let Some(coordinator) = self.spaces.read().get(space).cloned() else {
            return Ok(None);
        };
        let (modulations, total_modulations) =
            bound_recent_modulations(coordinator.field_modulations()?, requested_limit);
        Ok(Some(ConsciousFieldDiagnostics {
            space: space.clone(),
            indicators: coordinator.field_metric_indicators(),
            truncated: total_modulations > modulations.len(),
            total_modulations,
            modulations,
        }))
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
        for registration in self.processors(&space, recipient, root) {
            coordinator.register_bounded_processor(registration)?;
        }
        spaces.insert(space, coordinator.clone());
        Ok(coordinator)
    }

    fn processors(
        &self,
        space: &AgoraSpaceId,
        recipient: ProcessId,
        root: ProcessId,
    ) -> Vec<ProcessorRegistration> {
        let processors: Vec<(Arc<dyn ConsciousProcessor>, Vec<&str>, VisibilityScope)> = vec![
            (
                Arc::new(DomainProcessor::new(
                    space,
                    ProcessorKind::Dasein,
                    self.clock.clone(),
                )),
                vec!["aletheon.workspace.any/v1"],
                VisibilityScope::Session,
            ),
            (
                Arc::new(DomainProcessor::new(
                    space,
                    ProcessorKind::Cognit,
                    self.clock.clone(),
                )),
                vec!["aletheon.workspace.any/v1"],
                VisibilityScope::Session,
            ),
            (
                Arc::new(MnemosyneProcessor::new(
                    space,
                    self.clock.clone(),
                    self.memory_service.clone(),
                )),
                vec![
                    "aletheon.workspace.observation/v1",
                    "aletheon.workspace.agent-result/v1",
                    "aletheon.workspace.governed-action-outcome/v1",
                ],
                VisibilityScope::PrivateProcess {
                    process: crate::r#impl::conscious::processor_source(space, "mnemosyne"),
                },
            ),
            (
                Arc::new(MetacogProcessor::new(space, self.clock.clone())),
                vec!["aletheon.workspace.any/v1"],
                VisibilityScope::Session,
            ),
            (
                Arc::new(CorpusProcessor::new(
                    space,
                    self.clock.clone(),
                    self.skills.clone(),
                )),
                vec![
                    "aletheon.workspace.observation/v1",
                    "aletheon.workspace.goal/v1",
                    "aletheon.workspace.plan/v1",
                ],
                VisibilityScope::Session,
            ),
            (
                Arc::new(AgentAdapter::new(space, self.clock.clone())),
                vec![
                    "aletheon.workspace.observation/v1",
                    "aletheon.workspace.evidence/v1",
                    "aletheon.workspace.agent-result/v1",
                ],
                VisibilityScope::AgentTree { root },
            ),
        ];
        processors
            .into_iter()
            .map(
                |(processor, schemas, response_visibility)| ProcessorRegistration {
                    processor,
                    recipient,
                    agent_root: root,
                    schemas: schemas.into_iter().map(fabric::SchemaId::from).collect(),
                    capacity: self.core_config.max_candidates_per_processor,
                    deadline_ms: self.core_config.processor_timeout.as_millis() as u64,
                    response_visibility,
                },
            )
            .collect()
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
        Ok(Arc::new(
            ConsciousActionBridge::new(
                coordinator,
                source,
                root,
                self.clock.clone(),
                Duration::from_secs(60),
            )?
            .with_arbitration_mode(self.core_config.arbitration_mode),
        ))
    }
}

#[async_trait]
impl ConsciousTurnPort for ConsciousWorkspaceRegistry {
    async fn batch_planner(
        &self,
        space: AgoraSpaceId,
    ) -> anyhow::Result<Arc<dyn cognit::harness::BatchPlanner>> {
        let coordinator = self
            .spaces
            .read()
            .get(&space)
            .cloned()
            .context("conscious workspace has not observed a turn")?;
        Ok(Arc::new(ConsciousWorkspaceBatchPlanner {
            coordinator,
            mode: self.core_config.arbitration_mode,
        }))
    }

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
}

impl ProcessorKind {
    fn id(&self) -> &'static str {
        match self {
            Self::Dasein => "dasein",
            Self::Cognit => "cognit",
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
        let health = ProcessorHealth::Healthy;
        let detail = None;
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use uuid::Uuid;

    fn event(index: usize) -> fabric::ConsciousTraceEvent {
        fabric::ConsciousTraceEvent::Prediction {
            prediction_id: index.to_string(),
            surprised: false,
            outcome_ref: "test".into(),
        }
    }

    #[test]
    fn diagnostics_keep_only_requested_most_recent_events() {
        let (events, total) = bound_recent_modulations((0..5).map(event).collect(), 2);
        assert_eq!(total, 5);
        assert_eq!(events, vec![event(3), event(4)]);
    }

    #[test]
    fn diagnostics_enforce_global_bound_and_nonzero_limit() {
        let (events, _) = bound_recent_modulations(
            (0..MAX_DIAGNOSTIC_MODULATIONS + 10).map(event).collect(),
            usize::MAX,
        );
        assert_eq!(events.len(), MAX_DIAGNOSTIC_MODULATIONS);
        let (events, _) = bound_recent_modulations(vec![event(0), event(1)], 0);
        assert_eq!(events, vec![event(1)]);
    }

    #[test]
    fn batch_reorder_builds_complete_causal_modulation_trace() {
        let call = CapabilityCall {
            operation_id: fabric::OperationId(Uuid::from_u128(41)),
            process_id: ProcessId(Uuid::from_u128(42)),
            name: "file_read".into(),
            input: serde_json::json!({"path":"README.md"}),
            call_id: "call-reordered".into(),
            deadline: None,
        };
        let decision = CapabilityBatchDecision {
            call_id: call.call_id.clone(),
            decision: FieldDecisionKind::Reorder,
            reason: FieldDecisionReason::Selected,
            priority: 0.75,
            broadcast_epoch: Some(fabric::BroadcastEpoch(9)),
        };
        let trace = reorder_trace_event(
            ConsciousArbitrationMode::Observe,
            &call,
            &decision,
            fabric::BroadcastEpoch(9),
            "metric:9",
        );
        let fabric::ConsciousTraceEvent::FieldModulation {
            mode,
            decision,
            reason,
            operation_id,
            call_id,
            broadcast_epoch,
            effective,
            metric_ref,
            ..
        } = trace
        else {
            panic!("expected field modulation trace")
        };
        assert_eq!(mode, ConsciousArbitrationMode::Observe);
        assert_eq!(decision, FieldDecisionKind::Reorder);
        assert_eq!(reason, FieldDecisionReason::Selected);
        assert_eq!(operation_id, call.operation_id.0.to_string());
        assert_eq!(call_id, call.call_id);
        assert_eq!(broadcast_epoch, Some(9));
        assert_eq!(effective, Some(0.75));
        assert_eq!(metric_ref, "metric:9");
    }
}
