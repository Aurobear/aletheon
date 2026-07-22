//! RobotHarness state machine — bounded retry/replan with deterministic verification.

use serde::{Deserialize, Serialize};
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
                VerificationSignal::Retryable { remaining_retries } if *remaining_retries > 0 => {
                    Self::Retry
                }
                VerificationSignal::Retryable { .. } => Self::Replan,
                VerificationSignal::Replannable { remaining_replans } if *remaining_replans > 0 => {
                    Self::Replan
                }
                VerificationSignal::Replannable { .. } => Self::Recover,
                VerificationSignal::Unsafe => Self::SafeStop,
                VerificationSignal::Unknown {
                    remaining_retries, ..
                } if *remaining_retries > 0 => Self::Retry,
                VerificationSignal::Unknown { .. } => Self::SafeStop,
            },
            Self::Retry => Self::Execute,
            Self::Replan => Self::Plan,
            Self::Recover => Self::Settle,
            Self::Settle => Self::Completed,
            Self::SafeStop => Self::Failed,
            Self::Completed | Self::Failed => self,
        }
    }
}

#[derive(Debug, Clone)]
pub enum VerificationSignal {
    Matched,
    Retryable { remaining_retries: u32 },
    Replannable { remaining_replans: u32 },
    Unsafe,
    Unknown { remaining_retries: u32 },
}

/// Configuration for the RobotHarness state machine.
#[derive(Debug, Clone)]
pub struct RobotHarnessConfig {
    /// Maximum retries (hard cap: 1 for P3).
    pub max_retries: u32,
    /// Maximum replans (hard cap: 1 for P3).
    pub max_replans: u32,
}

impl Default for RobotHarnessConfig {
    fn default() -> Self {
        Self {
            max_retries: 1,
            max_replans: 1,
        }
    }
}
