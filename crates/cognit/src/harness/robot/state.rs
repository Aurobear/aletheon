//! RobotHarness state machine — bounded retry/replan with deterministic verification.

use ::contracts::types::embodiment::{DeviceId, SkillDescriptor, SkillId};
use ::contracts::types::expected_outcome::ExpectedOutcome;
use ::contracts::types::robot_failure::RobotFailureClass;
use ::contracts::types::world_state::WorldSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// RobotHarness states, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RobotState {
    Observe,
    Plan,
    Authorize,
    Execute,
    Verify,
    Retry,
    Replan,
    Recover,
    Settle,
    SafeStop,
    Completed,
    Failed,
}

impl fmt::Display for RobotState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Observe => write!(f, "observe"),
            Self::Plan => write!(f, "plan"),
            Self::Authorize => write!(f, "authorize"),
            Self::Execute => write!(f, "execute"),
            Self::Verify => write!(f, "verify"),
            Self::Retry => write!(f, "retry"),
            Self::Replan => write!(f, "replan"),
            Self::Recover => write!(f, "recover"),
            Self::Settle => write!(f, "settle"),
            Self::SafeStop => write!(f, "safe_stop"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl RobotState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    pub fn next(self, verification: &VerificationSignal) -> Self {
        match self {
            Self::Observe => Self::Plan,
            Self::Plan => Self::Authorize,
            Self::Authorize => Self::Execute,
            Self::Execute => Self::Verify,
            Self::Verify => match verification {
                VerificationSignal::Matched => Self::Settle,
                VerificationSignal::Retryable {
                    remaining_retries, ..
                } if *remaining_retries > 0 => Self::Retry,
                VerificationSignal::Retryable {
                    remaining_replans, ..
                } if *remaining_replans > 0 => Self::Replan,
                VerificationSignal::Retryable { .. } => Self::SafeStop,
                VerificationSignal::Replannable { remaining_replans } if *remaining_replans > 0 => {
                    Self::Replan
                }
                VerificationSignal::Replannable { .. } => Self::SafeStop,
                VerificationSignal::Unsafe => Self::SafeStop,
                VerificationSignal::Unknown => Self::SafeStop,
            },
            Self::Retry => Self::Execute,
            Self::Replan => Self::Authorize,
            Self::Recover => Self::SafeStop,
            Self::Settle => Self::Completed,
            Self::SafeStop => Self::Failed,
            Self::Completed | Self::Failed => self,
        }
    }
}

#[derive(Debug, Clone)]
pub enum VerificationSignal {
    Matched,
    Retryable {
        remaining_retries: u32,
        remaining_replans: u32,
    },
    Replannable {
        remaining_replans: u32,
    },
    Unsafe,
    Unknown,
}

/// Bounded, typed summary of one completed execution/verification attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptSummary {
    pub attempt: u32,
    pub skill: SkillId,
    pub parameters_digest: String,
    pub snapshot_schema: String,
    pub snapshot_schema_version: u16,
    pub snapshot_sequence: u64,
    pub failure_class: RobotFailureClass,
    pub operation_id: Option<String>,
}

/// Finite replan input. Policy receives typed prior-attempt facts and remaining
/// host budgets, never an unstructured failure transcript.
#[derive(Debug, Clone)]
pub struct ReplanContext {
    pub goal: String,
    pub device: DeviceId,
    pub latest_snapshot: WorldSnapshot,
    pub failure_class: RobotFailureClass,
    pub completed_attempts: Vec<AttemptSummary>,
    pub allowed_skills: Vec<SkillDescriptor>,
    pub retries_remaining: u32,
    pub replans_remaining: u32,
}

/// Configuration for the RobotHarness state machine.
#[derive(Debug, Clone)]
pub struct RobotHarnessConfig {
    /// Maximum retries (hard cap: 1 for P3).
    pub max_retries: u32,
    /// Maximum replans (hard cap: 1 for P3).
    pub max_replans: u32,
    /// Fallback expected outcome when the Plan state produces no valid proposal.
    /// `None` means fail closed — never silently use a hardcoded predicate.
    pub default_expected_outcome: Option<ExpectedOutcome>,
    /// Maximum number of host-selected frame references presented to Policy.
    pub perception_max_frames: usize,
    /// Per-skill typed perception requirements. An empty/missing entry means
    /// that the skill is permitted to run without visual input.
    pub required_perception: BTreeMap<SkillId, Vec<(String, u16)>>,
}

impl Default for RobotHarnessConfig {
    fn default() -> Self {
        Self {
            max_retries: 1,
            max_replans: 1,
            default_expected_outcome: None,
            perception_max_frames: 4,
            required_perception: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_and_replan_budgets_are_independent() {
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Retryable {
                remaining_retries: 1,
                remaining_replans: 1,
            }),
            RobotState::Retry
        );
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Retryable {
                remaining_retries: 0,
                remaining_replans: 1,
            }),
            RobotState::Replan
        );
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Retryable {
                remaining_retries: 0,
                remaining_replans: 0,
            }),
            RobotState::SafeStop
        );
    }

    #[test]
    fn replanned_proposal_returns_to_authorization() {
        assert_eq!(
            RobotState::Replan.next(&VerificationSignal::Matched),
            RobotState::Authorize
        );
    }

    #[test]
    fn unsafe_unknown_and_exhausted_replan_stop_without_execution() {
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Unsafe),
            RobotState::SafeStop
        );
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Unknown),
            RobotState::SafeStop
        );
        assert_eq!(
            RobotState::Verify.next(&VerificationSignal::Replannable {
                remaining_replans: 0,
            }),
            RobotState::SafeStop
        );
    }
}
