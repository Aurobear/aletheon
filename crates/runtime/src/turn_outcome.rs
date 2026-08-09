//! R1 unified typed Turn Outcome (Aletheon closure plan §9).
//!
//! One turn's completion/stop/cancel/block/failure has unambiguous typed
//! semantics.  These are the canonical Runtime Turn outcome types (RA-04
//! aligns with them): the terminal is authoritative, never inferred from text
//! or EOF, and no compatibility path swallows an error or forges a permission.

use crate::event::TurnTerminal;
use crate::ids::TurnId;
use serde::{Deserialize, Serialize};

/// Why a completed turn stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    TurnComplete,
    MaxTurns,
    Interrupted,
}

/// Why a turn was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    UserRequested,
    Timeout,
    Disconnect,
    SystemShutdown,
}

/// Why a turn was blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    ApprovalRequired,
    PolicyDenied,
    ResourceUnavailable,
}

/// A typed turn failure.  Never swallowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFailure {
    pub category: String,
    pub message: String,
}

/// The unambiguous turn outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOutcome {
    Completed { stop_reason: StopReason },
    Cancelled { reason: CancelReason },
    Blocked { reason: BlockReason },
    Failed { error: TurnFailure },
}

/// Token usage for a turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// The unified typed turn execution result (closure plan §9 shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnExecutionResult {
    pub turn_id: TurnId,
    pub outcome: TurnOutcome,
    pub usage: TurnUsage,
    pub committed_event: String,
}

impl TurnExecutionResult {
    /// Map the typed outcome to the authoritative terminal (RA-04).  There is
    /// no terminal-inference path here.
    pub fn terminal(&self) -> TurnTerminal {
        match &self.outcome {
            TurnOutcome::Completed { .. } => TurnTerminal::Completed,
            TurnOutcome::Cancelled { .. } => TurnTerminal::Interrupted,
            TurnOutcome::Blocked { .. } => TurnTerminal::Interrupted,
            TurnOutcome::Failed { error } => TurnTerminal::Failed {
                message: error.message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_maps_to_terminal_completed() {
        let result = TurnExecutionResult {
            turn_id: TurnId("t1".into()),
            outcome: TurnOutcome::Completed {
                stop_reason: StopReason::TurnComplete,
            },
            usage: TurnUsage {
                input_tokens: 1,
                output_tokens: 2,
                ..Default::default()
            },
            committed_event: "evt-1".into(),
        };
        assert_eq!(result.terminal(), TurnTerminal::Completed);
    }

    #[test]
    fn failed_maps_to_terminal_failed_with_message() {
        let result = TurnExecutionResult {
            turn_id: TurnId("t1".into()),
            outcome: TurnOutcome::Failed {
                error: TurnFailure {
                    category: "provider".into(),
                    message: "rejected".into(),
                },
            },
            usage: TurnUsage::default(),
            committed_event: "evt-1".into(),
        };
        assert_eq!(
            result.terminal(),
            TurnTerminal::Failed {
                message: "rejected".into()
            }
        );
    }
}
