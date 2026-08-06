//! Bounded RobotHarness — Observe, Plan, Authorize, Execute, Verify, Retry, Replan, Recover, Settle, SafeStop.
//! Cognit owns the state machine; it does NOT depend on Executive or Hardware.

pub mod proposal_validator;
pub mod session;
pub mod state;

use crate::harness::robot::proposal_validator::validate_proposal;
use crate::harness::robot::state::{
    AttemptSummary, ReplanContext, RobotHarnessConfig, RobotState, VerificationSignal,
};
use crate::ports::policy_provider::PolicyProviderPort;
use async_trait::async_trait;
use fabric::types::embodiment::{
    DeviceId, SkillDescriptor, SkillOutcome, SkillRequest, SkillResult,
};
use fabric::types::episode_report::{
    EpisodeSettlement, SafeStopOutcome, SafeStopReceipt, SettledEpisodeReport,
};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::frame::FrameRef;
use fabric::types::outcome_verification::VerificationReport;
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::robot_failure::{RobotFailure, RobotFailureClass};
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort, ANY_SCHEMA};
use fabric::OperationId;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Port for executing an embodied skill. Injected by Executive.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RobotExecutionError {
    #[error("execution rejected: {0}")]
    Rejected(String),
    #[error("embodiment provider disconnected: {0}")]
    ProviderDisconnected(String),
    #[error("execution control failed: {0}")]
    Control(String),
}

impl RobotExecutionError {
    fn failure_class(&self) -> RobotFailureClass {
        match self {
            Self::Rejected(_) => RobotFailureClass::ExecutionRejected,
            Self::ProviderDisconnected(_) => RobotFailureClass::ProviderDisconnected,
            Self::Control(_) => RobotFailureClass::ExecutionFailed,
        }
    }
}

#[async_trait]
pub trait EmbodiedExecutionPort: Send + Sync {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, RobotExecutionError>;
    async fn cancel(&self, device: &DeviceId) -> Result<(), RobotExecutionError>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<(), RobotExecutionError>;
}

/// Abstract, bounded perception source. Implementations own freshness, URI
/// authority and byte-budget enforcement; Cognit only consumes validated refs.
#[async_trait]
pub trait RobotPerceptionPort: Send + Sync {
    async fn latest(
        &self,
        device: &DeviceId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String>;
}

/// Explicit no-visual source for skills that do not require perception.
pub struct NoopRobotPerception;

#[async_trait]
impl RobotPerceptionPort for NoopRobotPerception {
    async fn latest(
        &self,
        _device: &DeviceId,
        _after_sequence: Option<u64>,
        _limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String> {
        Ok(Vec::new())
    }
}

/// Port for verifying outcomes. Injected by Executive.
/// The `device` lets the verifier wait for post-execution observations via the
/// world state (`observe_until`) to satisfy a continuous stable window.
#[async_trait]
pub trait OutcomeVerifierPort: Send + Sync {
    async fn verify(
        &self,
        expected: &ExpectedOutcome,
        device: &DeviceId,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        attempt: u32,
    ) -> VerificationReport;
}

/// Port for episode persistence. Injected.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait EpisodeSink: Send + Sync {
    async fn append_attempt(
        &self,
        episode_id: &str,
        attempt: u32,
        attempt_id: &str,
        operation_id: Option<&OperationId>,
        request: &SkillRequest,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String>;
    async fn close_episode(
        &self,
        episode_id: &str,
        settlement: EpisodeSettlement,
    ) -> Result<(), String>;
    /// Record the post-execution verification for a previously appended attempt.
    /// Attempts are appended before execution completes; the verification lands
    /// later (Verify step), so it is persisted as an update to keep the durable
    /// record authoritative for report building and the promotion gate.
    async fn update_verification(
        &self,
        episode_id: &str,
        attempt_id: &str,
        after: Option<&WorldSnapshot>,
        verification: &VerificationReport,
    ) -> Result<(), String>;
    /// Load an episode's recorded attempts in order for report building. The
    /// durable sink is authoritative — the terminal harness state only carries
    /// the latest attempt.
    async fn load_attempts(
        &self,
        episode_id: &str,
    ) -> Result<Vec<fabric::types::episode_report::AttemptRecord>, String>;
    /// Atomically publish the final immutable report receipt. Implementations
    /// that cannot persist reports must remain explicit no-ops; production uses
    /// the SQLite sink below this port.
    async fn store_settled_report(&self, _report: &SettledEpisodeReport) -> Result<(), String> {
        Ok(())
    }
    async fn load_settled_report(
        &self,
        _episode_id: &str,
    ) -> Result<Option<SettledEpisodeReport>, String> {
        Ok(None)
    }
}

/// Port for distilling a settled, matched robot episode into long-term memory.
/// The immutable receipt is verified again by the production adapter.
#[async_trait]
pub trait EpisodePromotionPort: Send + Sync {
    async fn promote(&self, report: &SettledEpisodeReport) -> Result<(), String>;
}

/// Governance audit boundary for the immutable episode receipt. Implementations
/// may hash-chain or externally journal it, but never receive a mutable report.
#[async_trait]
pub trait EpisodeAuditPort: Send + Sync {
    async fn record(&self, report: &SettledEpisodeReport) -> Result<(), String>;
}

/// No-op promoter for tests and unconfigured compositions — promotion stays
/// absent rather than silently failing.
pub struct NoopEpisodePromotion;

#[async_trait]
impl EpisodePromotionPort for NoopEpisodePromotion {
    async fn promote(&self, _report: &SettledEpisodeReport) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RobotHarnessState {
    pub state: RobotState,
    pub device: DeviceId,
    pub goal: String,
    pub episode_id: String,
    pub attempt: u32,
    pub retries_used: u32,
    pub replans_used: u32,
    pub latest_snapshot: Option<WorldSnapshot>,
    pub latest_skill_request: Option<SkillRequest>,
    pub latest_skill_result: Option<SkillResult>,
    pub latest_verification: Option<VerificationReport>,
    /// Expected outcome carried from the validated policy proposal.
    pub latest_expected_outcome: Option<ExpectedOutcome>,
    /// Accepted Policy identity, with provider/protocol bound by the host-side
    /// gateway adapter rather than inferred from model prose.
    pub latest_policy_provenance: Option<PolicyProvenance>,
    /// Host/provider-issued operation id of the latest executed attempt.
    pub latest_operation_id: Option<OperationId>,
    /// Host-owned frame provenance attached to the accepted proposal/report.
    pub latest_frame_refs: Vec<FrameRef>,
    /// Ordered, de-duplicated frame provenance across every accepted attempt.
    /// This keeps earlier retry/replan inputs in the final report without ever
    /// retaining image bytes.
    pub episode_frame_refs: Vec<FrameRef>,
    pub latest_perception_sequence: Option<u64>,
    /// Ordered typed failure history; the primary cause is never overwritten by
    /// later safe-stop or settlement failures.
    pub failures: Vec<RobotFailure>,
    /// Terminal result returned by the safe-stop executor boundary. This is
    /// recorded after the call completes and is never inferred from settlement.
    pub safe_stop: Option<SafeStopReceipt>,
    pub completed_attempts: Vec<AttemptSummary>,
    /// Set exactly once by the durable close boundary and used as the only
    /// authority for both report settlement and `TurnStop` projection.
    pub settlement: Option<EpisodeSettlement>,
}

pub struct RobotHarness {
    config: RobotHarnessConfig,
    world_state: Arc<dyn WorldStatePort>,
    executor: Arc<dyn EmbodiedExecutionPort>,
    verifier: Arc<dyn OutcomeVerifierPort>,
    episodes: Arc<dyn EpisodeSink>,
    policy: Arc<dyn PolicyProviderPort>,
    perception: Arc<dyn RobotPerceptionPort>,
    allowed_skills: Vec<SkillDescriptor>,
}

/// Resolve a schema-qualified expected-outcome tree to its one observation
/// stream. Different schemas own independent sequence counters; using a
/// device-global "latest" snapshot for before/after would make EpisodeReport
/// ordering depend on whichever camera/state source happened to poll last.
fn expected_observation_schema(expected: &ExpectedOutcome) -> &str {
    fn visit<'a>(predicate: &'a OutcomePredicate, selected: &mut Option<&'a str>) -> bool {
        match predicate {
            OutcomePredicate::Equals { path, .. }
            | OutcomePredicate::NotEquals { path, .. }
            | OutcomePredicate::Range { path, .. }
            | OutcomePredicate::Change { path, .. } => {
                let Some(segment) = path.split('.').next().filter(|segment| !segment.is_empty())
                else {
                    return false;
                };
                match selected {
                    Some(existing) => *existing == segment,
                    slot @ None => {
                        *slot = Some(segment);
                        true
                    }
                }
            }
            OutcomePredicate::All { predicates } | OutcomePredicate::Any { predicates } => {
                !predicates.is_empty()
                    && predicates
                        .iter()
                        .all(|predicate| visit(predicate, selected))
            }
        }
    }

    let mut selected = None;
    if visit(&expected.predicate, &mut selected) {
        selected.unwrap_or(ANY_SCHEMA)
    } else {
        ANY_SCHEMA
    }
}

impl RobotHarness {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: RobotHarnessConfig,
        world_state: Arc<dyn WorldStatePort>,
        executor: Arc<dyn EmbodiedExecutionPort>,
        verifier: Arc<dyn OutcomeVerifierPort>,
        episodes: Arc<dyn EpisodeSink>,
        policy: Arc<dyn PolicyProviderPort>,
        perception: Arc<dyn RobotPerceptionPort>,
        allowed_skills: Vec<SkillDescriptor>,
    ) -> Self {
        Self {
            config,
            world_state,
            executor,
            verifier,
            episodes,
            policy,
            perception,
            allowed_skills,
        }
    }

    /// Durable episode sink accessor — used to rebuild the full attempt list
    /// for report building (the terminal state only carries the latest one).
    pub fn episodes(&self) -> Arc<dyn EpisodeSink> {
        self.episodes.clone()
    }

    pub fn init(&self, device: DeviceId, goal: String, episode_id: String) -> RobotHarnessState {
        RobotHarnessState {
            state: RobotState::Observe,
            device,
            goal,
            episode_id,
            attempt: 0,
            retries_used: 0,
            replans_used: 0,
            latest_snapshot: None,
            latest_skill_request: None,
            latest_skill_result: None,
            latest_verification: None,
            latest_expected_outcome: None,
            latest_policy_provenance: None,
            latest_operation_id: None,
            latest_frame_refs: Vec::new(),
            episode_frame_refs: Vec::new(),
            latest_perception_sequence: None,
            failures: Vec::new(),
            safe_stop: None,
            completed_attempts: Vec::new(),
            settlement: None,
        }
    }

    /// Resolve the expected outcome before execution. Fail closed when neither
    /// the validated proposal nor configuration supplied one.
    fn resolve_expected(&self, state: &RobotHarnessState) -> Result<ExpectedOutcome, String> {
        state
            .latest_expected_outcome
            .clone()
            .or(self.config.default_expected_outcome.clone())
            .ok_or_else(|| "no expected outcome available (proposal carried none)".to_string())
    }

    fn fail(state: &mut RobotHarnessState, class: RobotFailureClass, detail: impl Into<String>) {
        state.failures.push(RobotFailure::new(class, detail));
    }

    async fn select_perception(
        &self,
        state: &RobotHarnessState,
    ) -> Result<Vec<PerceptionObservation>, RobotFailure> {
        let visual = self
            .perception
            .latest(
                &state.device,
                state.latest_perception_sequence,
                self.config.perception_max_frames,
            )
            .await
            .map_err(|reason| {
                RobotFailure::new(
                    RobotFailureClass::PerceptionUnavailable,
                    format!("perception selection failed: {reason}"),
                )
            })?;
        validate_selected_perception(&state.device, &visual, self.config.perception_max_frames)
            .map_err(|reason| {
                RobotFailure::new(RobotFailureClass::PerceptionUnavailable, reason)
            })?;
        if visual.is_empty()
            && !self.allowed_skills.is_empty()
            && self.allowed_skills.iter().all(|descriptor| {
                self.config
                    .required_perception
                    .get(&descriptor.skill)
                    .is_some_and(|requirements| !requirements.is_empty())
            })
        {
            return Err(RobotFailure::new(
                RobotFailureClass::PerceptionUnavailable,
                "required perception is unavailable for every allowed skill",
            ));
        }
        Ok(visual)
    }

    fn accept_proposals(
        &self,
        state: &mut RobotHarnessState,
        proposals: &[SkillProposal],
        visual: &[PerceptionObservation],
        snapshots: &[WorldSnapshot],
    ) -> Result<(), String> {
        let selected_frames = visual
            .iter()
            .map(|observation| observation.frame.clone())
            .collect::<Vec<_>>();
        // Policy-input frame provenance remains relevant even when every
        // proposal is rejected before execution. Retain only Host-selected
        // references; model output never owns these fields.
        state.latest_frame_refs.clone_from(&selected_frames);
        for frame in &selected_frames {
            if !state
                .episode_frame_refs
                .iter()
                .any(|existing| existing.uri == frame.uri)
            {
                state.episode_frame_refs.push(frame.clone());
            }
        }

        // A rejected decision is still a Policy decision and needs auditable
        // model provenance. Record it only when every structurally valid
        // proposal agrees on the same Host-bound identity; mixed or malformed
        // identities remain absent rather than being guessed.
        if let Some(first) = proposals
            .first()
            .filter(|proposal| proposal.validate().is_ok())
        {
            if proposals.iter().all(|proposal| {
                proposal.validate().is_ok() && proposal.provenance == first.provenance
            }) {
                state.latest_policy_provenance = Some(first.provenance.clone());
            }
        }
        let mut rejection_reasons = Vec::new();
        for (index, proposal) in proposals.iter().enumerate() {
            // Frame provenance is host-owned input evidence, not a
            // model-controlled response field.
            let mut proposal = proposal.clone();
            proposal.frame_refs.clone_from(&selected_frames);
            let validation =
                validate_proposal(&proposal, &state.device, &self.allowed_skills, snapshots);
            let perception_available = required_perception_available(
                &proposal.skill,
                visual,
                &self.config.required_perception,
            );
            if validation.is_ok() && perception_available {
                state.latest_expected_outcome = Some(proposal.expected_outcome.clone());
                state.latest_policy_provenance = Some(proposal.provenance.clone());
                state.latest_frame_refs = proposal.frame_refs.clone();
                for frame in &proposal.frame_refs {
                    if !state
                        .episode_frame_refs
                        .iter()
                        .any(|existing| existing.uri == frame.uri)
                    {
                        state.episode_frame_refs.push(frame.clone());
                    }
                }
                state.latest_perception_sequence = visual
                    .iter()
                    .map(|observation| observation.frame.frame_id)
                    .max();
                state.latest_skill_request = Some(SkillRequest {
                    skill: proposal.skill,
                    device: proposal.device,
                    parameters: proposal.parameters,
                });
                return Ok(());
            }
            if let Err(errors) = validation {
                let reason = errors
                    .iter()
                    .take(4)
                    .map(|error| format!("{}: {}", error.field, error.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                rejection_reasons.push(format!("proposal[{index}] {reason}"));
            } else {
                rejection_reasons.push(format!(
                    "proposal[{index}] required perception contract is unsatisfied"
                ));
            }
        }
        Err(if proposals.is_empty() {
            "policy_empty_response: gateway returned no proposals".into()
        } else {
            format!(
                "policy_all_proposals_rejected: {}",
                rejection_reasons
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        })
    }

    fn record_attempt_summary(&self, state: &mut RobotHarnessState, class: RobotFailureClass) {
        let (Some(request), Some(snapshot)) = (
            state.latest_skill_request.as_ref(),
            state.latest_snapshot.as_ref(),
        ) else {
            return;
        };
        state.completed_attempts.push(AttemptSummary {
            attempt: state.attempt,
            skill: request.skill.clone(),
            parameters_digest: parameters_digest(&request.parameters),
            snapshot_schema: snapshot.schema.clone(),
            snapshot_schema_version: snapshot.schema_version,
            snapshot_sequence: snapshot.sequence,
            failure_class: class,
            operation_id: state
                .latest_operation_id
                .as_ref()
                .map(|operation| operation.0.to_string()),
        });
    }

    fn repeated_in_unchanged_world(&self, state: &RobotHarnessState) -> bool {
        let (Some(request), Some(snapshot)) = (
            state.latest_skill_request.as_ref(),
            state.latest_snapshot.as_ref(),
        ) else {
            return false;
        };
        let digest = parameters_digest(&request.parameters);
        state.completed_attempts.iter().any(|attempt| {
            attempt.skill == request.skill
                && attempt.parameters_digest == digest
                && attempt.snapshot_schema == snapshot.schema
                && attempt.snapshot_schema_version == snapshot.schema_version
                && attempt.snapshot_sequence == snapshot.sequence
        })
    }

    fn route_replan_or_stop(&self, state: &mut RobotHarnessState) {
        if state.replans_used < self.config.max_replans {
            state.state = RobotState::Replan;
        } else {
            Self::fail(
                state,
                RobotFailureClass::ReplanBudgetExhausted,
                "replan budget exhausted",
            );
            state.state = RobotState::SafeStop;
        }
    }

    async fn persist_attempt(
        &self,
        state: &RobotHarnessState,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
    ) -> Result<(), String> {
        let attempt_id = state
            .latest_operation_id
            .as_ref()
            .map(|operation| operation.0.to_string())
            .unwrap_or_else(|| format!("attempt:{}-{}", state.episode_id, state.attempt));
        self.episodes
            .append_attempt(
                &state.episode_id,
                state.attempt,
                &attempt_id,
                state.latest_operation_id.as_ref(),
                state.latest_skill_request.as_ref().ok_or_else(|| {
                    "cannot persist robot attempt without governed skill request".to_string()
                })?,
                expected,
                before,
                None,
                state.latest_skill_result.as_ref(),
                None,
            )
            .await
    }

    async fn safe_stop_and_close(&self, state: &mut RobotHarnessState) {
        let trigger = state.failures.last().map(|failure| failure.class);
        let outcome = match self.executor.safe_stop(&state.device).await {
            Ok(()) => SafeStopOutcome::Succeeded,
            Err(error) => {
                Self::fail(
                    state,
                    RobotFailureClass::SafeStopFailure,
                    format!("safe stop failed: {error}"),
                );
                SafeStopOutcome::Failed
            }
        };
        state.safe_stop = Some(SafeStopReceipt {
            attempted_after_attempt: state.attempt,
            trigger,
            outcome,
        });
        let settlement = if state
            .failures
            .iter()
            .any(|failure| failure.class == RobotFailureClass::Cancelled)
        {
            EpisodeSettlement::Cancelled
        } else {
            EpisodeSettlement::Failed
        };
        if let Err(reason) = self
            .episodes
            .close_episode(&state.episode_id, settlement)
            .await
        {
            Self::fail(
                state,
                RobotFailureClass::SettlementFailure,
                format!("failed episode settlement failed: {reason}"),
            );
        }
        state.settlement = Some(settlement);
        state.state = RobotState::Failed;
    }

    /// Cancel an active episode through the execution port, then require a safe
    /// stop and failed settlement. Secondary failures append to the original
    /// `Cancelled` fact rather than replacing it.
    pub async fn cancel_and_safe_stop(&self, mut state: RobotHarnessState) -> RobotHarnessState {
        Self::fail(
            &mut state,
            RobotFailureClass::Cancelled,
            "robot episode cancelled by caller",
        );
        if let Err(error) = self.executor.cancel(&state.device).await {
            Self::fail(
                &mut state,
                error.failure_class(),
                format!("cancel propagation failed: {error}"),
            );
        }
        self.safe_stop_and_close(&mut state).await;
        state
    }

    pub async fn step(&self, mut state: RobotHarnessState) -> RobotHarnessState {
        match state.state {
            RobotState::Observe => {
                let snapshot = self.world_state.latest(&state.device, ANY_SCHEMA).await;
                match snapshot {
                    Some(snapshot) if !snapshot.stale => {
                        state.latest_snapshot = Some(snapshot);
                        state.state = state.state.next(&VerificationSignal::Matched);
                    }
                    _ => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::ObservationUnavailable,
                            "no fresh observation available before planning",
                        );
                        state.state = RobotState::SafeStop;
                    }
                }
            }
            RobotState::Plan => {
                // Planning needs one candidate per schema. Choosing only the
                // device-global freshest sample makes policy validity depend on
                // whichever independent sensor stream happened to publish last.
                let snapshots = self
                    .world_state
                    .latest_all(&state.device)
                    .await
                    .into_iter()
                    .filter(|snapshot| !snapshot.stale)
                    .collect::<Vec<_>>();
                if snapshots.is_empty() {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::ObservationUnavailable,
                        "no fresh observation schemas available for planning",
                    );
                    state.state = RobotState::SafeStop;
                    return state;
                }
                let visual = match self.select_perception(&state).await {
                    Ok(visual) => visual,
                    Err(failure) => {
                        state.failures.push(failure);
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                let proposals = match self
                    .policy
                    .propose(
                        &state.goal,
                        &state.device,
                        &snapshots,
                        &visual,
                        &self.allowed_skills,
                    )
                    .await
                {
                    Ok(proposals) => proposals,
                    Err(error) => {
                        Self::fail(&mut state, error.failure_class(), error.to_string());
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                match self.accept_proposals(&mut state, &proposals, &visual, &snapshots) {
                    Ok(()) => state.state = state.state.next(&VerificationSignal::Matched),
                    Err(reason) => {
                        Self::fail(&mut state, RobotFailureClass::ProposalRejected, reason);
                        state.state = RobotState::SafeStop;
                    }
                }
            }
            RobotState::Authorize => {
                // Executive's Kernel remains the permit/lease authority. Replans
                // return here so every new proposal traverses authorization.
                state.state = state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Execute => {
                let Some(request) = state.latest_skill_request.clone() else {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::ProposalRejected,
                        "no skill request available after proposal validation",
                    );
                    state.state = RobotState::SafeStop;
                    return state;
                };
                let expected = match self.resolve_expected(&state) {
                    Ok(expected) => expected,
                    Err(reason) => {
                        Self::fail(&mut state, RobotFailureClass::ProposalRejected, reason);
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                let schema = expected_observation_schema(&expected);
                let before = self
                    .world_state
                    .latest(&state.device, schema)
                    .await
                    .or_else(|| state.latest_snapshot.clone());
                state.latest_snapshot = before.clone();
                state.attempt = state.attempt.saturating_add(1);
                state.latest_operation_id = None;
                state.latest_skill_result = None;
                state.latest_verification = None;
                let submitted_request = request.clone();
                match self.executor.execute(request).await {
                    Ok(result) => {
                        state.latest_operation_id = Some(result.operation_id);
                        state.latest_skill_result = Some(result.clone());
                        if result.skill != submitted_request.skill
                            || result.device != submitted_request.device
                        {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::ProviderDisconnected,
                                format!(
                                    "embodiment result identity mismatch: requested={}/{} returned={}/{}",
                                    submitted_request.device.0,
                                    submitted_request.skill.0,
                                    result.device.0,
                                    result.skill.0
                                ),
                            );
                            if let Err(reason) = self
                                .persist_attempt(&state, &expected, before.as_ref())
                                .await
                            {
                                Self::fail(
                                    &mut state,
                                    RobotFailureClass::PersistenceFailure,
                                    format!("append mismatched attempt failed: {reason}"),
                                );
                            }
                            self.record_attempt_summary(
                                &mut state,
                                RobotFailureClass::ProviderDisconnected,
                            );
                            state.state = RobotState::SafeStop;
                            return state;
                        }
                        let outcome_failure = match &result.outcome {
                            SkillOutcome::Succeeded => None,
                            SkillOutcome::Failed { reason } => Some((
                                RobotFailureClass::ExecutionFailed,
                                format!("skill execution failed: {reason}"),
                            )),
                            SkillOutcome::Cancelled => Some((
                                RobotFailureClass::Cancelled,
                                "skill execution returned cancelled".into(),
                            )),
                            SkillOutcome::TimedOut => Some((
                                RobotFailureClass::ExecutionTimedOut,
                                "skill execution timed out".into(),
                            )),
                        };
                        if let Some((class, detail)) = &outcome_failure {
                            Self::fail(&mut state, *class, detail.clone());
                        }
                        if let Err(reason) = self
                            .persist_attempt(&state, &expected, before.as_ref())
                            .await
                        {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::PersistenceFailure,
                                format!("append attempt failed: {reason}"),
                            );
                            state.state = RobotState::SafeStop;
                            return state;
                        }
                        match outcome_failure {
                            None => state.state = state.state.next(&VerificationSignal::Matched),
                            Some((class, _)) => {
                                self.record_attempt_summary(&mut state, class);
                                if matches!(class, RobotFailureClass::ExecutionFailed) {
                                    self.route_replan_or_stop(&mut state);
                                } else {
                                    state.state = RobotState::SafeStop;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let class = error.failure_class();
                        Self::fail(&mut state, class, error.to_string());
                        let persistence_failed = if let Err(reason) = self
                            .persist_attempt(&state, &expected, before.as_ref())
                            .await
                        {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::PersistenceFailure,
                                format!("append failed attempt failed: {reason}"),
                            );
                            true
                        } else {
                            false
                        };
                        self.record_attempt_summary(&mut state, class);
                        if persistence_failed || class == RobotFailureClass::ProviderDisconnected {
                            state.state = RobotState::SafeStop;
                        } else {
                            self.route_replan_or_stop(&mut state);
                        }
                    }
                }
            }
            RobotState::Verify => {
                let expected = match self.resolve_expected(&state) {
                    Ok(expected) => expected,
                    Err(reason) => {
                        Self::fail(&mut state, RobotFailureClass::ProposalRejected, reason);
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                let after = self
                    .world_state
                    .latest(&state.device, expected_observation_schema(&expected))
                    .await;
                let report = self
                    .verifier
                    .verify(
                        &expected,
                        &state.device,
                        state.latest_snapshot.as_ref(),
                        after.as_ref(),
                        state.attempt,
                    )
                    .await;
                state.latest_verification = Some(report.clone());
                let attempt_id = state
                    .latest_operation_id
                    .as_ref()
                    .map(|operation| operation.0.to_string())
                    .unwrap_or_else(|| format!("attempt:{}-{}", state.episode_id, state.attempt));
                let persistence_error = self
                    .episodes
                    .update_verification(&state.episode_id, &attempt_id, after.as_ref(), &report)
                    .await
                    .err();
                if let Some(snapshot) = after {
                    state.latest_snapshot = Some(snapshot);
                }

                let remaining_retries = self.config.max_retries.saturating_sub(state.retries_used);
                let remaining_replans = self.config.max_replans.saturating_sub(state.replans_used);
                use fabric::types::outcome_verification::VerificationDecision;
                let signal = match report.decision {
                    VerificationDecision::Matched => VerificationSignal::Matched,
                    VerificationDecision::RetryableMismatch => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::VerificationTimeout,
                            "verification stable window timed out",
                        );
                        self.record_attempt_summary(
                            &mut state,
                            RobotFailureClass::VerificationTimeout,
                        );
                        if remaining_retries == 0 {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::RetryBudgetExhausted,
                                "retry budget exhausted",
                            );
                        }
                        if remaining_retries == 0 && remaining_replans == 0 {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::ReplanBudgetExhausted,
                                "replan budget exhausted",
                            );
                        }
                        VerificationSignal::Retryable {
                            remaining_retries,
                            remaining_replans,
                        }
                    }
                    VerificationDecision::ReplannableMismatch => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::VerificationMismatch,
                            "verification mismatch requires replanning",
                        );
                        self.record_attempt_summary(
                            &mut state,
                            RobotFailureClass::VerificationMismatch,
                        );
                        if remaining_replans == 0 {
                            Self::fail(
                                &mut state,
                                RobotFailureClass::ReplanBudgetExhausted,
                                "replan budget exhausted",
                            );
                        }
                        VerificationSignal::Replannable { remaining_replans }
                    }
                    VerificationDecision::Unsafe => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::Unsafe,
                            "unsafe verification predicate matched",
                        );
                        self.record_attempt_summary(&mut state, RobotFailureClass::Unsafe);
                        VerificationSignal::Unsafe
                    }
                    VerificationDecision::Unknown => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::VerificationTimeout,
                            "verification evidence was unavailable or stale",
                        );
                        self.record_attempt_summary(
                            &mut state,
                            RobotFailureClass::VerificationTimeout,
                        );
                        VerificationSignal::Unknown
                    }
                };
                if let Some(reason) = persistence_error {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::PersistenceFailure,
                        format!("verification persistence failed: {reason}"),
                    );
                    state.state = RobotState::SafeStop;
                } else {
                    state.state = state.state.next(&signal);
                }
            }
            RobotState::Retry => {
                if state.retries_used >= self.config.max_retries {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::RetryBudgetExhausted,
                        "retry budget exhausted before retry execution",
                    );
                    state.state = RobotState::SafeStop;
                } else {
                    state.retries_used += 1;
                    state.state = state.state.next(&VerificationSignal::Matched);
                }
            }
            RobotState::Replan => {
                if state.replans_used >= self.config.max_replans {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::ReplanBudgetExhausted,
                        "replan budget exhausted before policy request",
                    );
                    state.state = RobotState::SafeStop;
                    return state;
                }
                let Some(latest_snapshot) = state.latest_snapshot.clone() else {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::ObservationUnavailable,
                        "replan requires a latest world snapshot",
                    );
                    state.state = RobotState::SafeStop;
                    return state;
                };
                if latest_snapshot.stale {
                    Self::fail(
                        &mut state,
                        RobotFailureClass::ObservationUnavailable,
                        "replan snapshot is stale",
                    );
                    state.state = RobotState::SafeStop;
                    return state;
                }
                let visual = match self.select_perception(&state).await {
                    Ok(visual) => visual,
                    Err(failure) => {
                        state.failures.push(failure);
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                state.replans_used += 1;
                let context = ReplanContext {
                    goal: state.goal.clone(),
                    device: state.device.clone(),
                    latest_snapshot: latest_snapshot.clone(),
                    failure_class: state
                        .completed_attempts
                        .last()
                        .map(|attempt| attempt.failure_class)
                        .or_else(|| state.failures.last().map(|failure| failure.class))
                        .unwrap_or(RobotFailureClass::VerificationMismatch),
                    completed_attempts: state.completed_attempts.clone(),
                    allowed_skills: self.allowed_skills.clone(),
                    retries_remaining: self.config.max_retries.saturating_sub(state.retries_used),
                    replans_remaining: self.config.max_replans.saturating_sub(state.replans_used),
                };
                let proposals = match self.policy.replan(&context, &visual).await {
                    Ok(proposals) => proposals,
                    Err(error) => {
                        Self::fail(&mut state, error.failure_class(), error.to_string());
                        state.state = RobotState::SafeStop;
                        return state;
                    }
                };
                let snapshots = vec![latest_snapshot];
                match self.accept_proposals(&mut state, &proposals, &visual, &snapshots) {
                    Ok(()) if self.repeated_in_unchanged_world(&state) => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::RepeatedFailure,
                            "replan repeated the same skill and parameters in an unchanged world",
                        );
                        state.state = RobotState::SafeStop;
                    }
                    Ok(()) => state.state = state.state.next(&VerificationSignal::Matched),
                    Err(reason) => {
                        Self::fail(&mut state, RobotFailureClass::ProposalRejected, reason);
                        state.state = RobotState::SafeStop;
                    }
                }
            }
            RobotState::Recover => {
                state.state = state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Settle => {
                match self
                    .episodes
                    .close_episode(&state.episode_id, EpisodeSettlement::Completed)
                    .await
                {
                    Ok(()) => {
                        state.settlement = Some(EpisodeSettlement::Completed);
                        state.state = state.state.next(&VerificationSignal::Matched);
                    }
                    Err(reason) => {
                        Self::fail(
                            &mut state,
                            RobotFailureClass::SettlementFailure,
                            format!("completed episode settlement failed: {reason}"),
                        );
                        state.state = RobotState::SafeStop;
                    }
                }
            }
            RobotState::SafeStop => self.safe_stop_and_close(&mut state).await,
            RobotState::Completed | RobotState::Failed => {}
        }
        state
    }
}

fn parameters_digest(parameters: &serde_json::Value) -> String {
    fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => serde_json::to_value(
                map.iter()
                    .map(|(key, value)| (key.clone(), canonicalize(value)))
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
            .expect("BTreeMap JSON serialization is infallible"),
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(canonicalize).collect())
            }
            other => other.clone(),
        }
    }
    let bytes = serde_json::to_vec(&canonicalize(parameters))
        .expect("serde_json::Value serialization is infallible");
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_selected_perception(
    device: &DeviceId,
    visual: &[PerceptionObservation],
    limit: usize,
) -> Result<(), String> {
    if limit > 4 || visual.len() > limit {
        return Err(format!(
            "perception selection exceeds proposal frame limit: {} > {}",
            visual.len(),
            limit
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for observation in visual {
        observation.validate()?;
        if &observation.device != device {
            return Err(format!(
                "perception device mismatch: expected={} actual={}",
                device.0, observation.device.0
            ));
        }
        if !seen.insert(observation.frame.uri.as_str()) {
            return Err(format!(
                "duplicate perception frame URI: {}",
                observation.frame.uri
            ));
        }
    }
    Ok(())
}

fn required_perception_available(
    skill: &fabric::types::embodiment::SkillId,
    visual: &[PerceptionObservation],
    requirements: &std::collections::BTreeMap<
        fabric::types::embodiment::SkillId,
        Vec<(String, u16)>,
    >,
) -> bool {
    requirements.get(skill).is_none_or(|required| {
        required.iter().all(|(schema, version)| {
            visual.iter().any(|observation| {
                &observation.schema == schema && observation.schema_version == *version
            })
        })
    })
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    fn expected(predicate: OutcomePredicate) -> ExpectedOutcome {
        ExpectedOutcome {
            predicate,
            freshness_ms: 500,
            stable_window_ms: 3_000,
            timeout_ms: 9_000,
        }
    }

    #[test]
    fn qualified_outcome_selects_one_sequence_authority() {
        let expected = expected(OutcomePredicate::All {
            predicates: vec![
                OutcomePredicate::Range {
                    path: "base_pose.position.z".into(),
                    min: Some(0.8),
                    max: Some(1.05),
                },
                OutcomePredicate::Range {
                    path: "base_pose.orientation.y".into(),
                    min: Some(-0.15),
                    max: Some(0.15),
                },
            ],
        });
        assert_eq!(expected_observation_schema(&expected), "base_pose");
    }

    #[test]
    fn mixed_or_unqualified_outcome_uses_any_schema_compatibility() {
        let mixed = expected(OutcomePredicate::All {
            predicates: vec![
                OutcomePredicate::Equals {
                    path: "base_pose.mode".into(),
                    value: serde_json::json!("standing"),
                },
                OutcomePredicate::Range {
                    path: "base_twist.linear_velocity.x".into(),
                    min: Some(-0.01),
                    max: Some(0.01),
                },
            ],
        });
        assert_eq!(expected_observation_schema(&mixed), ANY_SCHEMA);
        assert_eq!(
            expected_observation_schema(&expected(OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("standing"),
            })),
            "mode"
        );
    }
}
