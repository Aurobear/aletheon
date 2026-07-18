//! Durable write tracking and bounded writer failure (M4-T1).
//!
//! Every persistent write through the turn coordinator is tracked by phase.
//! When `grok_hardening.compaction_v2` is enabled the coordinator refuses to
//! remove an active turn from the index unless its terminal flush succeeded,
//! leaving it visible to the M4-T2 recovery scan.

use std::fmt;

/// Which phase of the turn lifecycle produced a write result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePhase {
    SessionCreate,
    UserMessage,
    ToolCall,
    ToolResult,
    ContextProjection,
    TerminalFlush,
    ContextFragment,
}

impl fmt::Display for WritePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::SessionCreate => "session_create",
            Self::UserMessage => "user_message",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::ContextProjection => "context_projection",
            Self::TerminalFlush => "terminal_flush",
            Self::ContextFragment => "context_fragment",
        };
        f.write_str(label)
    }
}

/// Outcome of a single bounded write.
#[derive(Debug, Clone)]
pub enum WriteResult {
    /// Write succeeded and data is durable.
    Succeeded,
    /// Write failed with a bounded error. The caller should treat the
    /// turn as unrecoverable and leave the active index entry intact so
    /// the M4-T2 recovery scan can classify it.
    Failed { reason: String, phase: WritePhase },
}

impl WriteResult {
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Accumulator for per-turn write outcomes.
#[derive(Debug, Clone, Default)]
pub struct TurnWriteTracker {
    results: Vec<WriteResult>,
}

impl TurnWriteTracker {
    pub fn new() -> Self {
        Self {
            results: Vec::with_capacity(8),
        }
    }

    pub fn record(&mut self, result: WriteResult) {
        self.results.push(result);
    }

    /// True when every recorded write succeeded. A single failure is
    /// enough to mark the turn as not-durable.
    pub fn all_succeeded(&self) -> bool {
        !self.results.iter().any(|r| r.is_failed())
    }

    pub fn into_results(self) -> Vec<WriteResult> {
        self.results
    }
}

/// Convert an anyhow error from an append call into a bounded
/// `WriteResult` at the given phase.
pub fn write_failed(error: &anyhow::Error, phase: WritePhase) -> WriteResult {
    WriteResult::Failed {
        reason: format!("{error:#}"),
        phase,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tracker_is_all_succeeded() {
        assert!(TurnWriteTracker::new().all_succeeded());
    }

    #[test]
    fn single_failure_marks_not_durable() {
        let mut tracker = TurnWriteTracker::new();
        tracker.record(WriteResult::Succeeded);
        tracker.record(WriteResult::Failed {
            reason: "disk full".into(),
            phase: WritePhase::TerminalFlush,
        });
        assert!(!tracker.all_succeeded());
    }

    #[test]
    fn write_phase_display_is_stable() {
        assert_eq!(WritePhase::TerminalFlush.to_string(), "terminal_flush");
        assert_eq!(WritePhase::UserMessage.to_string(), "user_message");
    }
}
