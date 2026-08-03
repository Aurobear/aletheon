//! `CognitiveSession` adapter over the RobotHarness state machine.
//!
//! Bridges the turn contract (`TurnRequest` / `TurnServices` / `TurnEventSink`)
//! to RobotHarness's own ports (policy / world / executor / verifier / episodes).
//! The turn input is the natural-language goal; the episode id is the turn's
//! operation id; the session drives `step()` until a terminal state.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::types::embodiment::DeviceId;
use fabric::{Clock, TurnEvent, TurnEventSink, TurnMetrics, TurnRequest, TurnResult, TurnStop};
use tokio_util::sync::CancellationToken;

use crate::harness::robot::state::RobotState;
use crate::harness::robot::{RobotHarness, RobotHarnessState};
use crate::harness::session::{CognitiveSession, CognitError};

/// Cognitive-session adapter for a single robot task.
pub struct RobotCognitiveSession {
    harness: RobotHarness,
    clock: Arc<dyn Clock>,
    cancellation: CancellationToken,
    device: DeviceId,
}

impl RobotCognitiveSession {
    pub fn new(
        harness: RobotHarness,
        clock: Arc<dyn Clock>,
        cancellation: CancellationToken,
        device: DeviceId,
    ) -> Self {
        Self {
            harness,
            clock,
            cancellation,
            device,
        }
    }

    fn outcome_summary(&self, state: &RobotHarnessState) -> String {
        let decision = state
            .latest_verification
            .as_ref()
            .map(|report| format!("{:?}", report.decision))
            .unwrap_or_else(|| "none".into());
        format!(
            "robot episode {} ({}): state={:?}, verification={}",
            state.episode_id, state.goal, state.state, decision
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

        let output = self.outcome_summary(&state);
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
