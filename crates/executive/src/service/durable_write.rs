//! Durable write tracking and bounded writer failure (M4-T1).
//!
//! Every persistent write through the turn coordinator is tracked by phase.
//! When `grok_hardening.compaction_v2` is enabled the coordinator refuses to
//! remove an active turn from the index unless its terminal flush succeeded,
//! leaving it visible to the M4-T2 recovery scan.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

const HEALTH_FILE: &str = "bounded-writer-health.json";
const MAX_FAILURE_REASON_BYTES: usize = 512;
static HEALTH_ROOT: OnceLock<Mutex<PathBuf>> = OnceLock::new();

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WriterHealthSnapshot {
    pub recent_failures: u64,
    pub writes_succeeding: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_reason: Option<String>,
}

pub fn configure_writer_health(data_dir: &Path) {
    let lock = HEALTH_ROOT.get_or_init(|| Mutex::new(data_dir.to_path_buf()));
    if let Ok(mut root) = lock.lock() {
        *root = data_dir.to_path_buf();
    }
}

pub fn read_writer_health(data_dir: &Path) -> anyhow::Result<WriterHealthSnapshot> {
    let path = data_dir.join(HEALTH_FILE);
    if !path.exists() {
        return Ok(WriterHealthSnapshot {
            writes_succeeding: true,
            ..WriterHealthSnapshot::default()
        });
    }
    let bytes = std::fs::read(&path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn record_writer_success() {
    let Some(root) = HEALTH_ROOT.get().and_then(|lock| lock.lock().ok()) else {
        return;
    };
    let path = root.join(HEALTH_FILE);
    let mut snapshot = read_writer_health(&root).unwrap_or_default();
    if snapshot.writes_succeeding {
        return;
    }
    snapshot.writes_succeeding = true;
    let temporary = path.with_extension("json.tmp");
    if let Ok(encoded) = serde_json::to_vec(&snapshot) {
        if std::fs::write(&temporary, encoded).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

fn record_writer_failure(reason: &str, phase: WritePhase) {
    let Some(root) = HEALTH_ROOT.get().and_then(|lock| lock.lock().ok()) else {
        return;
    };
    let path = root.join(HEALTH_FILE);
    let mut snapshot = read_writer_health(&root).unwrap_or_default();
    snapshot.recent_failures = snapshot.recent_failures.saturating_add(1);
    snapshot.writes_succeeding = false;
    snapshot.last_failure_phase = Some(phase.to_string());
    snapshot.last_failure_reason = Some(bounded_reason(reason));
    let temporary = path.with_extension("json.tmp");
    if let Ok(encoded) = serde_json::to_vec(&snapshot) {
        if std::fs::write(&temporary, encoded).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

fn bounded_reason(reason: &str) -> String {
    let end = reason
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= MAX_FAILURE_REASON_BYTES)
        .last()
        .unwrap_or(0);
    let end = if reason.len() <= MAX_FAILURE_REASON_BYTES {
        reason.len()
    } else {
        end
    };
    reason[..end].to_string()
}

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
    let reason = bounded_reason(&format!("{error:#}"));
    record_writer_failure(&reason, phase);
    WriteResult::Failed { reason, phase }
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

    #[test]
    fn failure_health_is_real_persisted_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        configure_writer_health(temp.path());
        let error = anyhow::anyhow!("{}", "x".repeat(2_000));
        let _ = write_failed(&error, WritePhase::TerminalFlush);
        let health = read_writer_health(temp.path()).unwrap();
        assert_eq!(health.recent_failures, 1);
        assert!(!health.writes_succeeding);
        assert_eq!(health.last_failure_phase.as_deref(), Some("terminal_flush"));
        assert!(health.last_failure_reason.unwrap().len() <= MAX_FAILURE_REASON_BYTES);
    }
}
