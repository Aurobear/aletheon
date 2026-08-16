//! `CognitiveSession` adapter over the RobotHarness state machine.
//!
//! Bridges the turn contract (`TurnRequest` / `TurnServices` / `TurnEventSink`)
//! to RobotHarness's own ports (policy / world / executor / verifier / episodes).
//! The turn input is the natural-language goal; the episode id is the turn's
//! operation id; the session drives `step()` until a terminal state.

use std::sync::{Arc, Mutex};

use ::contracts::types::embodiment::DeviceId;
use ::contracts::types::episode_report::{
    build_report, AttemptRecord, EpisodeArtifactManifest, EpisodeReport, EpisodeReportInput,
    SettledEpisodeReport,
};
use ::contracts::{Clock, TurnEvent, TurnEventSink, TurnMetrics, TurnRequest, TurnResult};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::harness::robot::{
    EpisodeAuditPort, EpisodePromotionPort, RobotHarness, RobotHarnessState,
};
use crate::harness::session::{CognitError, CognitiveSession};
use crate::harness::session_log::{
    HarnessSessionEventKind, HarnessSessionId, HarnessSessionLog, HarnessSessionPersistence,
    TurnEndReason,
};

/// Cognitive-session adapter for a single robot task.
///
/// The turn output is the structured `EpisodeReport` — the authoritative record
/// of the task. The natural-language answer is only a projection of it.
pub struct RobotCognitiveSession {
    harness: RobotHarness,
    clock: Arc<dyn Clock>,
    cancellation: CancellationToken,
    device: DeviceId,
    sim_scene_version: String,
    aletheon_commit: String,
    bridge_protocol_digest: String,
    skill_descriptor_digest: String,
    /// Distillation of settled+matched episodes into long-term memory. `None`
    /// in tests/unconfigured compositions (promotion stays absent).
    promoter: Option<Arc<dyn EpisodePromotionPort>>,
    auditor: Option<Arc<dyn EpisodeAuditPort>>,
    /// Optional diagnostic Harness turn envelope. Robot episode, settlement,
    /// audit and safety state remain authoritative even when this is attached.
    /// Production composition leaves it absent.
    session: Option<Arc<Mutex<HarnessSessionLog>>>,
}

impl RobotCognitiveSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        harness: RobotHarness,
        clock: Arc<dyn Clock>,
        cancellation: CancellationToken,
        device: DeviceId,
        sim_scene_version: impl Into<String>,
        aletheon_commit: impl Into<String>,
        bridge_protocol_digest: impl Into<String>,
        skill_descriptor_digest: impl Into<String>,
        promoter: Option<Arc<dyn EpisodePromotionPort>>,
    ) -> Self {
        Self {
            harness,
            clock,
            cancellation,
            device,
            sim_scene_version: sim_scene_version.into(),
            aletheon_commit: aletheon_commit.into(),
            bridge_protocol_digest: bridge_protocol_digest.into(),
            skill_descriptor_digest: skill_descriptor_digest.into(),
            promoter,
            auditor: None,
            session: None,
        }
    }

    /// Attach the governed audit sink without expanding the stable constructor
    /// used by tests and alternate compositions.
    pub fn with_auditor(mut self, auditor: Arc<dyn EpisodeAuditPort>) -> Self {
        self.auditor = Some(auditor);
        self
    }

    /// Attach a volatile diagnostic turn envelope for isolated tests.
    pub fn with_harness_session(mut self, id: HarnessSessionId) -> Result<Self, CognitError> {
        self.session = Some(Arc::new(Mutex::new(
            HarnessSessionLog::new(id).map_err(|error| CognitError::terminal(error.to_string()))?,
        )));
        Ok(self)
    }

    /// Attach an explicit diagnostic Harness log store. This does not replace
    /// Robot's episode, settlement, audit, or safe-stop authority.
    pub fn with_persistent_harness_session(
        mut self,
        id: HarnessSessionId,
        persistence: Arc<dyn HarnessSessionPersistence>,
    ) -> Result<Self, CognitError> {
        self.session = Some(Arc::new(Mutex::new(
            HarnessSessionLog::new_persistent(id, persistence)
                .map_err(|error| CognitError::terminal(error.to_string()))?,
        )));
        Ok(self)
    }

    /// Build the authoritative episode report from the terminal harness state.
    ///
    /// The durable episode sink is authoritative for the full attempt list — the
    /// terminal state only carries the latest attempt, so attempts are rebuilt
    /// from the sink and fall back to the in-memory latest when nothing is
    /// durable yet. Large artifacts (rosbag/log/plot) are referenced by their
    /// evidence refs from the executed skill and verification, never inlined.
    async fn build_report(&self, state: &RobotHarnessState) -> Result<EpisodeReport, CognitError> {
        let settlement = state.settlement.ok_or_else(|| {
            CognitError::terminal("robot terminal state has no authoritative settlement")
        })?;
        let durable = self
            .harness
            .episodes()
            .load_attempts(&state.episode_id)
            .await
            .map_err(|reason| {
                CognitError::terminal(format!("load durable robot attempts: {reason}"))
            })?;
        let attempts = if !durable.is_empty() {
            durable
        } else {
            state
                .latest_expected_outcome
                .as_ref()
                .filter(|_| state.attempt > 0)
                .map(|expected| {
                    let mut record = AttemptRecord::from_verification(
                        state.attempt,
                        state
                            .latest_operation_id
                            .as_ref()
                            .map(|op| op.0.to_string())
                            .unwrap_or_else(|| {
                                format!("attempt:{}-{}", state.episode_id, state.attempt)
                            }),
                        state
                            .latest_operation_id
                            .as_ref()
                            .map(|op| op.0.to_string()),
                        state.latest_skill_request.clone(),
                        expected.clone(),
                        state
                            .latest_skill_result
                            .as_ref()
                            .map(|result| format!("{:?}", result.outcome)),
                        state.latest_verification.as_ref(),
                        None,
                        None,
                        state
                            .latest_verification
                            .as_ref()
                            .map(|verification| verification.evaluated_sequence),
                    );
                    if let Some(result) = &state.latest_skill_result {
                        record.evidence_refs.extend(result.evidence.clone());
                    }
                    vec![record]
                })
                .unwrap_or_default()
        };
        let mut evidence_refs = attempts
            .iter()
            .flat_map(|attempt| attempt.evidence_refs.clone())
            .collect::<Vec<_>>();
        if let Some(result) = &state.latest_skill_result {
            evidence_refs.extend(result.evidence.clone());
        }
        if let Some(verification) = &state.latest_verification {
            evidence_refs.extend(verification.evidence.clone());
        }
        evidence_refs.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.uri.cmp(&right.uri))
        });
        evidence_refs.dedup_by(|left, right| left.kind == right.kind && left.uri == right.uri);
        let mut artifact_manifests = std::collections::BTreeMap::new();
        for frame in &state.episode_frame_refs {
            let manifest = EpisodeArtifactManifest::from_frame_ref(frame).map_err(|reason| {
                CognitError::terminal(format!("build frame artifact manifest: {reason}"))
            })?;
            artifact_manifests.insert((manifest.kind.clone(), manifest.uri.clone()), manifest);
        }
        for reference in &evidence_refs {
            let manifest = EpisodeArtifactManifest::from_legacy_evidence(reference);
            artifact_manifests
                .entry((manifest.kind.clone(), manifest.uri.clone()))
                .or_insert(manifest);
        }
        Ok(build_report(EpisodeReportInput {
            episode_id: state.episode_id.clone(),
            goal: state.goal.clone(),
            device: state.device.clone(),
            sim_scene_version: self.sim_scene_version.clone(),
            aletheon_commit: self.aletheon_commit.clone(),
            bridge_protocol_digest: self.bridge_protocol_digest.clone(),
            skill_descriptor_digest: self.skill_descriptor_digest.clone(),
            policy_provenance: state.latest_policy_provenance.clone(),
            failures: state.failures.clone(),
            safe_stop: state.safe_stop.clone(),
            selected_frames: state.episode_frame_refs.clone(),
            settlement,
            attempts,
            artifacts: artifact_manifests.into_values().collect(),
        }))
    }
}

impl RobotCognitiveSession {
    async fn execute_capability_turn(
        &mut self,
        request: &TurnRequest,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError> {
        let started = self.clock.mono_now();
        let mut state = self.harness.init(
            self.device.clone(),
            request.input.clone(),
            request.operation_id.0.to_string(),
        );
        if self.cancellation.is_cancelled() {
            state = self.harness.cancel_and_safe_stop(state).await;
        }

        // Drive the Observe -> Plan -> Authorize -> Execute -> Verify -> Settle
        // state machine to a terminal state. Cancellation races every step and
        // is recovered through cancel + safe-stop + failed settlement instead
        // of abandoning an active embodied operation.
        while !state.state.is_terminal() {
            let step_state = state.clone();
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => {
                    state = self.harness.cancel_and_safe_stop(state).await;
                }
                next = self.harness.step(step_state) => {
                    state = next;
                }
            }
        }

        let report = self.build_report(&state).await?;
        let settled = SettledEpisodeReport::new(report, self.clock.wall_now().0)
            .map_err(|reason| CognitError::terminal(format!("settle episode report: {reason}")))?;
        self.harness
            .episodes()
            .store_settled_report(&settled)
            .await
            .map_err(|reason| {
                CognitError::terminal(format!("persist settled episode report: {reason}"))
            })?;
        let report = settled.report();
        if let Some(auditor) = &self.auditor {
            auditor.record(&settled).await.map_err(|reason| {
                CognitError::terminal(format!("audit settled episode report: {reason}"))
            })?;
        }
        // Distill only a matched + settled episode into long-term memory; failed
        // episodes keep their evidence but are never promoted.
        if settled
            .can_promote()
            .map_err(|reason| CognitError::terminal(format!("verify settled report: {reason}")))?
        {
            if let Some(promoter) = &self.promoter {
                if let Err(reason) = promoter.promote(&settled).await {
                    tracing::warn!(
                        episode_id = %report.episode_id,
                        error = %reason,
                        "episode promotion failed"
                    );
                }
            }
        }
        // Only the immutable, already-persisted receipt enters the canonical
        // turn/session projection. The rendered JSON below remains a user-facing
        // projection and is never reparsed to recover authority facts.
        events
            .emit(TurnEvent::RobotEpisodeSettled {
                receipt: Box::new(settled.clone()),
            })
            .await;
        let output = serde_json::to_string(&report)
            .map_err(|e| CognitError::terminal(format!("report serialization: {e}")))?;
        let stop = report.settlement.turn_stop();
        let completed =
            report.settlement == ::contracts::types::episode_report::EpisodeSettlement::Completed;
        let elapsed_ms = self.clock.mono_now().0.saturating_sub(started.0);
        let result = TurnResult {
            output,
            stop: stop.clone(),
            failure: None,
            usage: Default::default(),
            metrics: TurnMetrics {
                tool_calls_made: 0,
                tool_errors: 0,
                provider_retries: 0,
                elapsed_ms,
                iterations: state.attempt as usize,
                completed_normally: completed,
            },
        };
        Ok(result)
    }
}

#[async_trait]
impl CognitiveSession for RobotCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        _services: &dyn ::contracts::TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;
        let harness_turn = if let Some(session) = &self.session {
            let mut session = session
                .lock()
                .map_err(|_| CognitError::terminal("robot Harness session lock poisoned"))?;
            let turn = session.next_turn_number();
            session
                .append(
                    self.clock.wall_now().0,
                    HarnessSessionEventKind::TurnStart { turn },
                    None,
                    vec![],
                )
                .map_err(|error| CognitError::terminal(error.to_string()))?;
            Some(turn)
        } else {
            None
        };

        let result = self.execute_capability_turn(&request, events).await;
        if let (Some(session), Some(turn)) = (&self.session, harness_turn) {
            let reason = match &result {
                Ok(result) => match result.stop {
                    ::contracts::TurnStop::Completed => TurnEndReason::Completed,
                    ::contracts::TurnStop::Blocked => TurnEndReason::Blocked,
                    ::contracts::TurnStop::Cancelled => TurnEndReason::Aborted {
                        cause: "robot capability cancelled".into(),
                    },
                    ::contracts::TurnStop::Failed => TurnEndReason::Error {
                        code: "robot_capability_failed".into(),
                        message: "robot capability returned failed settlement".into(),
                    },
                },
                Err(error) => TurnEndReason::Error {
                    code: "robot_capability_error".into(),
                    message: error.to_string(),
                },
            };
            session
                .lock()
                .map_err(|_| CognitError::terminal("robot Harness session lock poisoned"))?
                .append(
                    self.clock.wall_now().0,
                    HarnessSessionEventKind::TurnEnd { turn, reason },
                    None,
                    vec![],
                )
                .map_err(|error| CognitError::terminal(error.to_string()))?;
        }
        let stop = result
            .as_ref()
            .map(|result| result.stop.clone())
            .unwrap_or(::contracts::TurnStop::Failed);
        events
            .emit(TurnEvent::Finished {
                operation_id: request.operation_id,
                stop,
            })
            .await;
        result
    }
}
