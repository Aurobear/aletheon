//! Executive owner of the recurrent Dasein–Agora workspace loop.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use agora::{
    AdmissionOutcome, BroadcastCoordinator, CandidatePool, CandidatePoolConfig,
    SqliteBroadcastStore,
};
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use fabric::dasein::SelfTransitionReceipt;
use fabric::{
    AgoraSpaceId, BroadcastEpoch, BroadcastIntegrationReceipt, Clock, ConsciousArbitrationMode,
    ConsciousContextProjection, ConsciousFieldReadout, ConsciousProcessor, ConsciousTraceEvent,
    ContentId, ContextProjectionReceipt, FieldMetricHistory, FieldMetricIndicators,
    FieldMetricSnapshot, GoalFrame, LatestConsciousContextPort, MonoDeadline, MonoTime,
    OperationKind, OperationRequest, PredictionErrorFrame, PredictionFrame, ProcessId,
    ProcessorContext, ProcessorHealth, ProcessorId, ProcessorResponse, SalienceVector, SchemaId,
    SelectionExplanation, SelectionResult, VisibilityScope, WorkspaceBroadcast, WorkspaceCandidate,
    WorkspaceContent, WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use parking_lot::RwLock;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, CandidateSubmissionReceipt,
    ConsciousCandidatePort, ConsciousCycleReceipt, DaseinIntegration, DaseinWorkspacePort,
    ProcessorCycleStatus,
};

#[derive(Debug, Clone)]
pub struct ConsciousCoreConfig {
    pub arbitration_mode: ConsciousArbitrationMode,
    pub max_processors: usize,
    pub max_processor_concurrency: usize,
    pub max_candidates_per_processor: usize,
    pub max_recurrence_depth: u16,
    pub cycle_timeout: Duration,
    pub processor_timeout: Duration,
    pub candidate_ttl: Duration,
}

impl Default for ConsciousCoreConfig {
    fn default() -> Self {
        Self {
            arbitration_mode: ConsciousArbitrationMode::Observe,
            max_processors: 16,
            max_processor_concurrency: 4,
            max_candidates_per_processor: 8,
            max_recurrence_depth: 4,
            cycle_timeout: Duration::from_secs(10),
            processor_timeout: Duration::from_secs(2),
            candidate_ttl: Duration::from_secs(60),
        }
    }
}

impl ConsciousCoreConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=256).contains(&self.max_processors),
            "conscious processor capacity is invalid"
        );
        anyhow::ensure!(
            (1..=self.max_processors).contains(&self.max_processor_concurrency),
            "conscious processor concurrency is invalid"
        );
        anyhow::ensure!(
            (1..=fabric::MAX_PROCESSOR_RESPONSE_CANDIDATES)
                .contains(&self.max_candidates_per_processor),
            "processor candidate budget is invalid"
        );
        anyhow::ensure!(
            self.max_recurrence_depth > 0,
            "recurrence depth budget is zero"
        );
        anyhow::ensure!(
            !self.cycle_timeout.is_zero()
                && !self.processor_timeout.is_zero()
                && !self.candidate_ttl.is_zero(),
            "conscious timing budget is zero"
        );
        Ok(())
    }
}

#[derive(Clone)]
struct RegisteredProcessor {
    processor: Arc<dyn ConsciousProcessor>,
    recipient: ProcessId,
    agent_root: ProcessId,
    schemas: Vec<SchemaId>,
    capacity: usize,
    deadline: Duration,
    response_visibility: VisibilityScope,
}

#[derive(Clone)]
pub struct ProcessorRegistration {
    pub processor: Arc<dyn ConsciousProcessor>,
    pub recipient: ProcessId,
    pub agent_root: ProcessId,
    pub schemas: Vec<SchemaId>,
    pub capacity: usize,
    pub deadline_ms: u64,
    pub response_visibility: VisibilityScope,
}

pub struct ConsciousCoreCoordinator {
    space: AgoraSpaceId,
    pool: Mutex<CandidatePool>,
    broadcast: Arc<BroadcastCoordinator>,
    store: Arc<SqliteBroadcastStore>,
    dasein: Arc<dyn DaseinWorkspacePort>,
    dasein_source: ProcessId,
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    config: ConsciousCoreConfig,
    processors: RwLock<BTreeMap<String, RegisteredProcessor>>,
    next_workspace_version: Mutex<u64>,
    predictions: Mutex<HashMap<ContentId, PredictionFrame>>,
    field_metrics: RwLock<FieldMetricHistory>,
    cycle: Mutex<()>,
}

impl ConsciousCoreCoordinator {
    pub fn space(&self) -> &AgoraSpaceId {
        &self.space
    }

    /// Resolve one durable winner from the exact workspace epoch.
    ///
    /// Governed action outcomes use this read-only check so caller-provided
    /// selection receipts cannot manufacture an authority edge.
    pub fn durable_selected_candidate(
        &self,
        epoch: BroadcastEpoch,
        candidate_id: ContentId,
    ) -> anyhow::Result<Option<WorkspaceCandidate>> {
        let replay = self.store.replay(&self.space)?;
        Ok(replay
            .into_iter()
            .find(|entry| entry.broadcast.epoch == epoch)
            .and_then(|entry| {
                entry.broadcast.selected.into_iter().find(|candidate| {
                    candidate.id == candidate_id
                        && entry.broadcast.winner_ids.contains(&candidate.id)
                })
            }))
    }

    /// Return the current content-free field indicators without exposing the
    /// coordinator's mutable history.
    pub fn field_metric_indicators(&self) -> FieldMetricIndicators {
        self.field_metrics.read().indicators()
    }

    /// Return a read-only copy of the bounded numeric snapshots for audit and
    /// acceptance evidence.
    pub fn field_metric_snapshots(&self) -> Vec<FieldMetricSnapshot> {
        self.field_metrics
            .read()
            .entries()
            .iter()
            .cloned()
            .collect()
    }

    /// Append pre-execution conscious modulation evidence to the workspace's
    /// checksum-protected store.
    pub fn record_field_modulation(&self, event: &ConsciousTraceEvent) -> anyhow::Result<()> {
        self.store.save_field_modulation(&self.space, event)
    }

    /// Return durable modulation evidence for audit and acceptance projections.
    pub fn field_modulations(&self) -> anyhow::Result<Vec<ConsciousTraceEvent>> {
        self.store.field_modulations(&self.space)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        space: AgoraSpaceId,
        pool_config: CandidatePoolConfig,
        broadcast: Arc<BroadcastCoordinator>,
        store: Arc<SqliteBroadcastStore>,
        dasein: Arc<dyn DaseinWorkspacePort>,
        dasein_source: ProcessId,
        kernel: Arc<KernelRuntime>,
        config: ConsciousCoreConfig,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        let replay = store.replay(&space)?;
        let next_workspace_version = replay
            .last()
            .map(|entry| entry.broadcast.workspace_version.saturating_add(1))
            .unwrap_or(1);
        Ok(Self {
            pool: Mutex::new(CandidatePool::new(space.clone(), pool_config)?),
            space,
            broadcast,
            store,
            dasein,
            dasein_source,
            clock: kernel.clock(),
            kernel,
            config,
            processors: RwLock::new(BTreeMap::new()),
            next_workspace_version: Mutex::new(next_workspace_version),
            predictions: Mutex::new(HashMap::new()),
            field_metrics: RwLock::new(FieldMetricHistory::default()),
            cycle: Mutex::new(()),
        })
    }

    pub fn register_processor(
        &self,
        processor: Arc<dyn ConsciousProcessor>,
        recipient: ProcessId,
        agent_root: ProcessId,
    ) -> anyhow::Result<()> {
        self.register_bounded_processor(ProcessorRegistration {
            processor,
            recipient,
            agent_root,
            schemas: vec![SchemaId("aletheon.workspace.any/v1".into())],
            capacity: self.config.max_candidates_per_processor,
            deadline_ms: duration_millis(self.config.processor_timeout),
            response_visibility: VisibilityScope::Session,
        })
    }

    pub fn register_bounded_processor(
        &self,
        registration: ProcessorRegistration,
    ) -> anyhow::Result<()> {
        let id = registration.processor.id();
        id.validate()?;
        anyhow::ensure!(
            !registration.schemas.is_empty(),
            "processor schemas are empty"
        );
        anyhow::ensure!(
            registration
                .schemas
                .iter()
                .all(|schema| !schema.0.trim().is_empty()),
            "processor schema is invalid"
        );
        anyhow::ensure!(
            (1..=self.config.max_candidates_per_processor).contains(&registration.capacity),
            "processor capacity is invalid"
        );
        anyhow::ensure!(registration.deadline_ms > 0, "processor deadline is zero");
        let mut processors = self.processors.write();
        anyhow::ensure!(
            !processors.contains_key(&id.0),
            "conscious processor is already registered"
        );
        anyhow::ensure!(
            processors.len() < self.config.max_processors,
            "conscious processor capacity exceeded"
        );
        processors.insert(
            id.0,
            RegisteredProcessor {
                processor: registration.processor,
                recipient: registration.recipient,
                agent_root: registration.agent_root,
                schemas: registration.schemas,
                capacity: registration.capacity,
                deadline: Duration::from_millis(registration.deadline_ms),
                response_visibility: registration.response_visibility,
            },
        );
        Ok(())
    }

    pub async fn run_cycle(
        &self,
        owner: ProcessId,
        recurrence_depth: u16,
    ) -> anyhow::Result<ConsciousCycleReceipt> {
        anyhow::ensure!(
            recurrence_depth <= self.config.max_recurrence_depth,
            "conscious recurrence depth exceeded"
        );
        let _cycle = self.cycle.lock().await;
        let opened_at = self.clock.mono_now();
        let cycle_millis = duration_millis(self.config.cycle_timeout);
        let deadline = MonoDeadline::after(opened_at, cycle_millis);
        let operation = self
            .kernel
            .submit_operation(OperationRequest {
                owner,
                parent: None,
                kind: OperationKind::ConsciousCycle,
                deadline: Some(deadline),
            })
            .await?;
        self.kernel.start_operation(operation.id).await?;

        let result = self
            .run_cycle_inner(operation.id, recurrence_depth, opened_at, deadline)
            .await;
        match result {
            Ok(receipt) => {
                self.kernel.succeed_operation(operation.id).await?;
                Ok(receipt)
            }
            Err(error) => {
                self.kernel
                    .fail_operation(operation.id, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    async fn run_cycle_inner(
        &self,
        operation_id: fabric::OperationId,
        recurrence_depth: u16,
        opened_at: MonoTime,
        deadline: MonoDeadline,
    ) -> anyhow::Result<ConsciousCycleReceipt> {
        self.ensure_before(deadline)?;
        self.remodulate_pending().await?;
        let dasein_version = self.dasein.self_view().await?.version;
        let mut pool = self.pool.lock().await;
        let selection = pool.select(self.clock.mono_now());
        if selection.selected.is_empty() {
            pool.record_no_ignition(&selection)?;
            return Ok(ConsciousCycleReceipt {
                operation_id,
                depth: recurrence_depth,
                opened_at,
                broadcast: None,
                dasein_transition: None,
                processors: vec![],
            });
        }
        self.ensure_before(deadline)?;
        let mut version = self.next_workspace_version.lock().await;
        let workspace_version = *version;
        let broadcast = self
            .broadcast
            .broadcast_selection(
                &mut pool,
                selection,
                dasein_version,
                workspace_version,
                self.clock.wall_now(),
                self.clock.wall_now(),
            )
            .await?;
        *version = version.saturating_add(1);
        drop(version);
        drop(pool);

        self.ensure_before(deadline)?;
        let integration = self.dasein.integrate_broadcast(&broadcast).await?;
        let durable_integration = BroadcastIntegrationReceipt {
            space: self.space.clone(),
            epoch: broadcast.epoch,
            broadcast_checksum: broadcast.checksum()?,
            operation_id,
            recurrence_depth,
            transition: integration.transition.clone(),
        };
        self.store.append_integration(&durable_integration)?;

        self.submit_dasein_candidates(&broadcast, &integration)
            .await?;
        self.submit_prediction_errors(&broadcast).await?;
        let processors = self
            .deliver_processors(
                &broadcast,
                integration.transition.current_version,
                recurrence_depth,
                deadline,
            )
            .await?;

        let projection = ConsciousContextProjection {
            latest_broadcast: Some(broadcast.clone()),
            self_view: integration.self_view.clone(),
            receipt: ContextProjectionReceipt {
                space: self.space.clone(),
                broadcast_epoch: Some(broadcast.epoch),
                workspace_version: Some(broadcast.workspace_version),
                dasein_version: integration.transition.current_version,
                content_ids: broadcast.winner_ids.clone(),
            },
        };
        projection.validate()?;
        self.store.save_context_projection(&projection)?;
        self.record_field_metric(&projection, &durable_integration.broadcast_checksum)?;

        Ok(ConsciousCycleReceipt {
            operation_id,
            depth: recurrence_depth,
            opened_at,
            broadcast: Some(broadcast),
            dasein_transition: Some(integration.transition),
            processors,
        })
    }

    fn record_field_metric(
        &self,
        projection: &ConsciousContextProjection,
        broadcast_checksum: &str,
    ) -> anyhow::Result<()> {
        let readout = ConsciousFieldReadout::from_projection(projection)?
            .ok_or_else(|| anyhow::anyhow!("completed broadcast has no field readout"))?;
        let broadcast = projection
            .latest_broadcast
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("completed projection has no broadcast"))?;
        let protention = broadcast
            .selected
            .iter()
            .filter(|candidate| matches!(&candidate.content, WorkspaceContent::Prediction(_)))
            .max_by(|left, right| {
                left.salience
                    .confidence
                    .partial_cmp(&right.salience.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let protention_salience = protention
            .map(|candidate| salience_values(candidate.salience))
            .unwrap_or([0.0; 8]);
        let protention_horizon_ms = protention.and_then(|candidate| match &candidate.content {
            WorkspaceContent::Prediction(prediction) => Some(prediction.horizon_ms),
            _ => None,
        });
        let action_salience = broadcast
            .selected
            .iter()
            .filter(|candidate| matches!(&candidate.content, WorkspaceContent::ActionProposal(_)))
            .max_by(|left, right| {
                left.salience
                    .confidence
                    .partial_cmp(&right.salience.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|candidate| salience_values(candidate.salience))
            .unwrap_or([0.0; 8]);
        let snapshot = FieldMetricSnapshot {
            broadcast_epoch: broadcast.epoch.0,
            dasein_version: projection.self_view.version.0,
            salience: salience_values(readout.salience),
            care_action: readout.care_action,
            concern_urgency: f64::from(readout.concern_urgency),
            update_delta: 0.0,
            protention_salience,
            protention_horizon_ms,
            action_salience,
            temporally_decayed_update: 0.0,
            temporality_decay_weight: None,
            prior_protention_action_alignment: None,
            trace_event_id: format!(
                "broadcast:{}:{}:{broadcast_checksum}",
                self.space.0, broadcast.epoch.0
            ),
        };
        self.field_metrics.write().push(snapshot)
    }

    async fn remodulate_pending(&self) -> anyhow::Result<()> {
        let pending = self.pool.lock().await.pending();
        let mut updates = Vec::with_capacity(pending.len());
        for candidate in pending {
            updates.push((
                candidate.id,
                self.dasein.modulate_salience(&candidate).await?,
            ));
        }
        let mut pool = self.pool.lock().await;
        for (id, salience) in updates {
            if pool.pending().iter().any(|candidate| candidate.id == id) {
                pool.update_salience(id, salience)?;
            }
        }
        Ok(())
    }

    async fn submit_dasein_candidates(
        &self,
        broadcast: &WorkspaceBroadcast,
        integration: &DaseinIntegration,
    ) -> anyhow::Result<()> {
        let causal_ref = format!(
            "dasein:{}:v{}",
            integration.transition.event_id.0, integration.transition.current_version.0
        );
        let mut contents = integration
            .transition
            .emitted
            .iter()
            .cloned()
            .map(WorkspaceContent::Concern)
            .collect::<Vec<_>>();
        contents.extend(
            integration
                .self_view
                .care_concerns
                .iter()
                .cloned()
                .map(WorkspaceContent::CareConcern),
        );
        if let Some(projection) = &integration.self_view.projection {
            contents.push(WorkspaceContent::Goal(GoalFrame {
                id: format!("dasein-projection-v{}", integration.self_view.version.0),
                summary: projection.clone(),
            }));
        }
        contents.extend(integration.self_view.protentions.iter().map(|content| {
            WorkspaceContent::Prediction(PredictionFrame {
                statement: content.clone(),
                horizon_ms: duration_millis(self.config.candidate_ttl),
            })
        }));
        for (index, content) in contents.into_iter().enumerate() {
            let candidate = self.dasein_candidate(
                broadcast,
                &integration.transition,
                index,
                content,
                causal_ref.clone(),
            );
            let _ = self
                .submit_candidate(CandidateSubmission {
                    candidate,
                    cause: CandidateCause::DaseinTransition {
                        event_id: integration.transition.event_id,
                        version: integration.transition.current_version,
                    },
                })
                .await?;
        }
        Ok(())
    }

    fn dasein_candidate(
        &self,
        broadcast: &WorkspaceBroadcast,
        transition: &SelfTransitionReceipt,
        index: usize,
        content: WorkspaceContent,
        causal_ref: String,
    ) -> WorkspaceCandidate {
        let now = self.clock.mono_now();
        WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: deterministic_content_id(&format!(
                "dasein:{}:{}:{index}",
                transition.event_id.0, transition.current_version.0
            )),
            space: self.space.clone(),
            source: self.dasein_source,
            turn: None,
            content,
            confidence: 1.0,
            salience: baseline_salience(0.65),
            provenance: WorkspaceProvenance {
                producer: self.dasein_source,
                operation: None,
                source_refs: vec![
                    causal_ref,
                    format!("broadcast:{}:{}", self.space.0, broadcast.epoch.0),
                ],
                observed_at: self.clock.wall_now(),
            },
            visibility: VisibilityScope::Session,
            dependencies: broadcast.winner_ids.clone(),
            created_at: now,
            expires_at: Some(MonoDeadline::after(
                now,
                duration_millis(self.config.candidate_ttl),
            )),
        }
    }

    async fn submit_prediction_errors(&self, broadcast: &WorkspaceBroadcast) -> anyhow::Result<()> {
        let mut predictions = self.predictions.lock().await;
        for candidate in &broadcast.selected {
            if let WorkspaceContent::Prediction(prediction) = &candidate.content {
                predictions.insert(candidate.id, prediction.clone());
            }
        }
        let mut errors = Vec::new();
        for outcome in &broadcast.selected {
            let magnitude = match &outcome.content {
                WorkspaceContent::ToolOutcome(value) => {
                    Some(if value.is_error { 1.0 } else { 0.0 })
                }
                WorkspaceContent::GovernedActionOutcome(value) => {
                    Some(if value.is_error { 1.0 } else { 0.0 })
                }
                _ => None,
            };
            let Some(magnitude) = magnitude else {
                continue;
            };
            for dependency in &outcome.dependencies {
                if let Some(prediction) = predictions.remove(dependency) {
                    errors.push((*dependency, outcome.id, prediction, magnitude));
                }
            }
        }
        drop(predictions);
        for (prediction_id, outcome_id, prediction, magnitude) in errors {
            let event_ref = format!("prediction_pair:{}:{}", prediction_id.0, outcome_id.0);
            let now = self.clock.mono_now();
            let candidate = WorkspaceCandidate {
                schema_version: WORKSPACE_SCHEMA_V1,
                id: deterministic_content_id(&event_ref),
                space: self.space.clone(),
                source: self.dasein_source,
                turn: None,
                content: WorkspaceContent::PredictionError(PredictionErrorFrame {
                    prediction_id,
                    description: format!(
                        "prediction '{}' compared with selected outcome {}",
                        prediction.statement, outcome_id.0
                    ),
                    magnitude,
                }),
                confidence: 1.0,
                salience: SalienceVector {
                    prediction_error: magnitude,
                    ..baseline_salience(0.6)
                },
                provenance: WorkspaceProvenance {
                    producer: self.dasein_source,
                    operation: None,
                    source_refs: vec![event_ref.clone()],
                    observed_at: self.clock.wall_now(),
                },
                visibility: VisibilityScope::Session,
                dependencies: vec![prediction_id, outcome_id],
                created_at: now,
                expires_at: Some(MonoDeadline::after(
                    now,
                    duration_millis(self.config.candidate_ttl),
                )),
            };
            let _ = self
                .submit_candidate(CandidateSubmission {
                    candidate,
                    cause: CandidateCause::ExternalObservation { event_ref },
                })
                .await?;
        }
        Ok(())
    }

    async fn deliver_processors(
        &self,
        broadcast: &WorkspaceBroadcast,
        dasein_version: fabric::dasein::SelfVersion,
        recurrence_depth: u16,
        deadline: MonoDeadline,
    ) -> anyhow::Result<Vec<ProcessorCycleStatus>> {
        let processors = self.processors.read().values().cloned().collect::<Vec<_>>();
        let semaphore = Arc::new(Semaphore::new(self.config.max_processor_concurrency));
        let mut tasks = JoinSet::new();
        for registration in processors {
            let Some(view) = processor_broadcast_view(
                broadcast,
                registration.recipient,
                registration.agent_root,
                &registration.schemas,
            )?
            else {
                continue;
            };
            let processor = registration.processor.clone();
            let processor_id = processor.id();
            let context = ProcessorContext {
                space: self.space.clone(),
                source_epoch: broadcast.epoch,
                dasein_version,
                recipient: registration.recipient,
                agent_root: registration.agent_root,
                recurrence_depth,
                deadline,
                max_candidates: registration.capacity,
            };
            let semaphore = semaphore.clone();
            let timeout = self.config.processor_timeout.min(registration.deadline);
            let response_visibility = registration.response_visibility.clone();
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                let response =
                    tokio::time::timeout(timeout, processor.on_broadcast(view, context.clone()))
                        .await;
                let response = match response {
                    Ok(response) if response.processor == processor_id => response,
                    Ok(_) => failed_response(
                        processor_id,
                        context.source_epoch,
                        "processor response identity mismatch",
                    ),
                    Err(_) => ProcessorResponse {
                        processor: processor_id,
                        source_epoch: context.source_epoch,
                        health: ProcessorHealth::TimedOut,
                        candidates: vec![],
                        acknowledgements: vec![],
                        detail: Some("processor deadline exceeded".into()),
                    },
                };
                anyhow::Ok((context, response, response_visibility))
            });
        }

        let mut statuses = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            self.ensure_before(deadline)?;
            let (context, response, response_visibility) = joined??;
            let response = match response.validate(&context) {
                Ok(())
                    if response
                        .candidates
                        .iter()
                        .all(|candidate| candidate.visibility == response_visibility) =>
                {
                    response
                }
                Ok(()) => failed_response(
                    response.processor,
                    context.source_epoch,
                    "processor response visibility exceeds registration",
                ),
                Err(error) => failed_response(
                    response.processor,
                    context.source_epoch,
                    &format!("invalid processor response: {error}"),
                ),
            };
            self.store.append_processor_response(&context, &response)?;
            let mut admitted = Vec::new();
            if response.health == ProcessorHealth::Healthy
                || response.health == ProcessorHealth::Degraded
            {
                for candidate in response.candidates.iter().cloned() {
                    let receipt = self
                        .submit_candidate(CandidateSubmission {
                            candidate,
                            cause: CandidateCause::ProcessorResponse {
                                processor: response.processor.clone(),
                                source_epoch: response.source_epoch,
                            },
                        })
                        .await?;
                    if receipt.status == CandidateAdmissionStatus::Accepted {
                        admitted.push(receipt.candidate_id);
                    }
                }
            }
            statuses.push(ProcessorCycleStatus {
                processor: response.processor,
                health: response.health,
                source_epoch: response.source_epoch,
                admitted_candidates: admitted,
                detail: response.detail,
            });
        }
        statuses.sort_by(|left, right| left.processor.0.cmp(&right.processor.0));
        Ok(statuses)
    }

    fn ensure_before(&self, deadline: MonoDeadline) -> anyhow::Result<()> {
        anyhow::ensure!(
            !deadline.is_expired_at(self.clock.mono_now()),
            "conscious cycle deadline exceeded"
        );
        Ok(())
    }
}

#[async_trait]
impl ConsciousCandidatePort for ConsciousCoreCoordinator {
    async fn submit_candidate(
        &self,
        mut submission: CandidateSubmission,
    ) -> anyhow::Result<CandidateSubmissionReceipt> {
        submission.validate()?;
        anyhow::ensure!(
            submission.candidate.space == self.space,
            "candidate targets another conscious workspace"
        );
        submission.candidate.salience =
            self.dasein.modulate_salience(&submission.candidate).await?;
        submission.candidate.validate()?;
        let id = submission.candidate.id;
        let mut pool = self.pool.lock().await;
        let outcome = CandidatePool::admit(&mut pool, submission.candidate, self.clock.mono_now());
        let (status, detail) = match outcome {
            AdmissionOutcome::Accepted { .. } => (CandidateAdmissionStatus::Accepted, None),
            AdmissionOutcome::Duplicate { existing } => (
                CandidateAdmissionStatus::Duplicate,
                Some(format!("duplicates candidate {}", existing.0)),
            ),
            AdmissionOutcome::RejectedCapacity => (
                CandidateAdmissionStatus::RejectedCapacity,
                Some("workspace candidate capacity exceeded".into()),
            ),
            AdmissionOutcome::RejectedSourceQuota { source } => (
                CandidateAdmissionStatus::RejectedSourceQuota,
                Some(format!("source quota exceeded for {}", source.0)),
            ),
            AdmissionOutcome::RejectedWrongSpace => (
                CandidateAdmissionStatus::RejectedWrongSpace,
                Some("candidate targets another workspace".into()),
            ),
            AdmissionOutcome::RejectedInvalid { reason } => {
                (CandidateAdmissionStatus::RejectedInvalid, Some(reason))
            }
        };
        Ok(CandidateSubmissionReceipt {
            candidate_id: id,
            status,
            detail,
        })
    }
}

#[async_trait]
impl LatestConsciousContextPort for ConsciousCoreCoordinator {
    async fn latest_context(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection> {
        anyhow::ensure!(space == &self.space, "context workspace is not owned here");
        if let Some(projection) = self.store.latest_context_projection(space)? {
            return Ok(projection);
        }
        let self_view = self.dasein.self_view().await?;
        let projection = ConsciousContextProjection {
            latest_broadcast: None,
            receipt: ContextProjectionReceipt {
                space: space.clone(),
                broadcast_epoch: None,
                workspace_version: None,
                dasein_version: self_view.version,
                content_ids: vec![],
            },
            self_view,
        };
        projection.validate()?;
        self.store.save_context_projection(&projection)?;
        Ok(projection)
    }
}

fn processor_broadcast_view(
    broadcast: &WorkspaceBroadcast,
    recipient: ProcessId,
    agent_root: ProcessId,
    schemas: &[SchemaId],
) -> anyhow::Result<Option<WorkspaceBroadcast>> {
    let selected = broadcast
        .selected
        .iter()
        .filter(|candidate| match candidate.visibility {
            VisibilityScope::Session => true,
            VisibilityScope::PrivateProcess { process } => process == recipient,
            VisibilityScope::AgentTree { root } => root == agent_root,
        })
        .filter(|candidate| {
            schemas.iter().any(|schema| {
                schema.0 == "aletheon.workspace.any/v1"
                    || schema.0 == workspace_schema(&candidate.content)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(None);
    }
    let ids = selected
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let id_set = ids.iter().copied().collect::<HashSet<_>>();
    let explanation = SelectionExplanation {
        policy_version: broadcast.selected_because.policy_version,
        evaluated: broadcast
            .selected_because
            .evaluated
            .iter()
            .filter(|score| id_set.contains(&score.id))
            .cloned()
            .collect(),
        selected_ids: ids,
        rejected_below_ignition: broadcast
            .selected_because
            .rejected_below_ignition
            .iter()
            .filter(|id| id_set.contains(id))
            .copied()
            .collect(),
    };
    let view = WorkspaceBroadcast::from_selection(
        broadcast.epoch,
        SelectionResult {
            selected,
            explanation,
        },
        broadcast.dasein_version,
        broadcast.workspace_version,
    )?;
    Ok(Some(view))
}

fn workspace_schema(content: &WorkspaceContent) -> &'static str {
    match content {
        WorkspaceContent::Observation(_) => "aletheon.workspace.observation/v1",
        WorkspaceContent::RecalledExperience(_) => "aletheon.workspace.recalled-experience/v1",
        WorkspaceContent::Evidence(_) => "aletheon.workspace.evidence/v1",
        WorkspaceContent::Hypothesis(_) => "aletheon.workspace.hypothesis/v1",
        WorkspaceContent::Prediction(_) => "aletheon.workspace.prediction/v1",
        WorkspaceContent::PredictionError(_) => "aletheon.workspace.prediction-error/v1",
        WorkspaceContent::Goal(_) => "aletheon.workspace.goal/v1",
        WorkspaceContent::Concern(_) => "aletheon.workspace.concern/v1",
        WorkspaceContent::CareConcern(_) => "aletheon.workspace.care-concern/v1",
        WorkspaceContent::Plan(_) => "aletheon.workspace.plan/v1",
        WorkspaceContent::ActionProposal(_) => "aletheon.workspace.action-proposal/v1",
        WorkspaceContent::ToolOutcome(_) => "aletheon.workspace.tool-outcome/v1",
        WorkspaceContent::GovernedActionOutcome(_) => {
            "aletheon.workspace.governed-action-outcome/v1"
        }
        WorkspaceContent::AgentResult(_) => "aletheon.workspace.agent-result/v1",
        WorkspaceContent::Reflection(_) => "aletheon.workspace.reflection/v1",
        WorkspaceContent::Extension { .. } => "aletheon.workspace.extension/v1",
    }
}

fn failed_response(
    processor: ProcessorId,
    epoch: BroadcastEpoch,
    detail: &str,
) -> ProcessorResponse {
    ProcessorResponse {
        processor,
        source_epoch: epoch,
        health: ProcessorHealth::Failed,
        candidates: vec![],
        acknowledgements: vec![],
        detail: Some(detail.chars().take(4096).collect()),
    }
}

fn baseline_salience(confidence: f32) -> SalienceVector {
    SalienceVector {
        urgency: 0.0,
        goal_relevance: 0.5,
        self_relevance: 0.5,
        novelty: 0.5,
        confidence,
        prediction_error: 0.0,
        affect_intensity: 0.0,
        social_relevance: 0.0,
    }
}

fn salience_values(salience: SalienceVector) -> [f64; 8] {
    salience.values().map(f64::from)
}

fn deterministic_content_id(material: &str) -> ContentId {
    ContentId(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        material.as_bytes(),
    ))
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
