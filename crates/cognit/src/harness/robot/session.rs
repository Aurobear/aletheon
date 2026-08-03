//! `CognitiveSession` adapter over the RobotHarness state machine.
//!
//! Bridges the turn contract (`TurnRequest` / `TurnServices` / `TurnEventSink`)
//! to RobotHarness's own ports (policy / world / executor / verifier / episodes).
//! The turn input is the natural-language goal; the episode id is the turn's
//! operation id; the session drives `step()` until a terminal state.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::types::embodiment::DeviceId;
use fabric::types::episode_report::{build_report, AttemptRecord, EpisodeReport};
use fabric::{Clock, TurnEvent, TurnEventSink, TurnMetrics, TurnRequest, TurnResult, TurnStop};
use tokio_util::sync::CancellationToken;

use crate::harness::robot::state::RobotState;
use crate::harness::robot::{RobotHarness, RobotHarnessState};
use crate::harness::session::{CognitiveSession, CognitError};

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
    ) -> Self {
        Self {
            harness,
            clock,
            cancellation,
            device,
            sim_scene_version: sim_scene_version.into(),
            aletheon_commit: aletheon_commit.into(),
            bridge_protocol_digest: bridge_protocol_digest.into(),
        }
    }

    /// Build the authoritative episode report from the terminal harness state.
    ///
    /// The durable episode sink is authoritative for the full attempt list — the
    /// terminal state only carries the latest attempt, so attempts are rebuilt
    /// from the sink and fall back to the in-memory latest when nothing is
    /// durable yet. Large artifacts (rosbag/log/plot) are referenced by their
    /// evidence refs from the executed skill and verification, never inlined.
    async fn build_report(&self, state: &RobotHarnessState) -> EpisodeReport {
        let settlement = if matches!(state.state, RobotState::Completed) {
            "completed"
        } else {
            "failed"
        };
        let attempts = match self
            .harness
            .episodes()
            .load_attempts(&state.episode_id)
            .await
        {
            Ok(durable) if !durable.is_empty() => durable,
            _ => state.latest_expected_outcome.as_ref().map(|expected| {
                vec![AttemptRecord::from_verification(
                    state.attempt,
                    state
                        .latest_operation_id
                        .as_ref()
                        .map(|op| op.0.to_string())
                        .unwrap_or_else(|| format!("attempt:{}-{}", state.episode_id, state.attempt)),
                    state.latest_operation_id.as_ref().map(|op| op.0.to_string()),
                    expected.clone(),
                    state
                        .latest_skill_result
                        .as_ref()
                        .map(|result| format!("{:?}", result.outcome)),
                    state.latest_verification.as_ref(),
                    None,
                )]
            }).unwrap_or_default(),
        };
        let mut artifacts = Vec::new();
        if let Some(result) = &state.latest_skill_result {
            artifacts.extend(result.evidence.clone());
        }
        if let Some(verification) = &state.latest_verification {
            artifacts.extend(verification.evidence.clone());
        }
        build_report(
            &state.episode_id,
            &state.goal,
            state.device.clone(),
            &self.sim_scene_version,
            &self.aletheon_commit,
            &self.bridge_protocol_digest,
            state.latest_snapshot.as_ref().map(|snap| snap.sequence),
            state.latest_verification.as_ref().map(|v| v.evaluated_sequence),
            settlement,
            attempts,
            artifacts,
        )
    }
}

#[async_trait]
impl CognitiveSession for RobotCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        _services: &dyn fabric::TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;
        if self.cancellation.is_cancelled() {
            return Err(CognitError::cancelled());
        }

        let started = self.clock.mono_now();
        let mut state = self.harness.init(
            self.device.clone(),
            request.input.clone(),
            request.operation_id.0.to_string(),
        );

        // Drive the Observe -> Plan -> Authorize -> Execute -> Verify -> Settle
        // state machine to a terminal state.
        while !state.state.is_terminal() {
            if self.cancellation.is_cancelled() {
                events
                    .emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Cancelled,
                    })
                    .await;
                return Err(CognitError::cancelled());
            }
            state = self.harness.step(state).await;
        }

        let report = self.build_report(&state).await;
        let output = serde_json::to_string(&report)
            .map_err(|e| CognitError::terminal(format!("report serialization: {e}")))?;
        let completed = matches!(state.state, RobotState::Completed);
        let stop = if completed {
            TurnStop::Completed
        } else {
            TurnStop::Blocked
        };
        let elapsed_ms = self.clock.mono_now().0.saturating_sub(started.0);
        let result = TurnResult {
            output,
            stop: stop.clone(),
            metrics: TurnMetrics {
                tool_calls_made: 0,
                tool_errors: 0,
                provider_retries: 0,
                elapsed_ms,
                iterations: state.attempt as usize,
                completed_normally: completed,
            },
        };
        events
            .emit(TurnEvent::Finished {
                operation_id: request.operation_id,
                stop,
            })
            .await;
        Ok(result)
    }
}
