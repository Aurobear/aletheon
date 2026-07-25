//! Append-only problem ledger — records and transitions problems via JSONL events.
//!
//! Events are appended to a JSONL file. Current state is rebuilt by replaying events.
//! Never rewrite old JSONL lines.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use super::fingerprint::problem_fingerprint;
use super::model::{ProblemRecord, ProblemSeverity, ProblemState, ProblemTransition};
use super::projection::Projection;

/// Errors that can occur during problem ledger operations.
#[derive(Debug, Error)]
pub enum ProblemError {
    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ProblemState,
        to: ProblemState,
    },

    #[error("problem with id {0} not found")]
    NotFound(String),

    #[error("problem with id {0} already exists")]
    AlreadyExists(String),

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("lock error: {0}")]
    Lock(String),
}

/// Input for observing a new problem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemFinding {
    /// Stable problem identifier.
    pub problem_id: String,
    /// Domain-neutral category.
    pub category: String,
    /// Domain-specific subtype.
    pub subtype: String,
    /// Domain where this problem was observed.
    pub domain: String,
    /// Subject (capability or component) affected.
    pub subject: String,
    /// Severity classification.
    pub severity: ProblemSeverity,
    /// Confidence in the problem classification (0-1_000 millis).
    pub confidence_millis: u16,
    /// Timestamp (ms since epoch) when observed.
    pub observed_at_ms: i64,
    /// Affected runtime versions.
    pub affected_versions: Vec<String>,
    /// Summary of what was expected.
    pub expected_summary: String,
    /// Summary of what was observed.
    pub observed_summary: String,
    /// Normalized failure signature for fingerprinting.
    pub failure_signature: String,
    /// Evidence references.
    pub evidence_ids: Vec<String>,
    /// Rubric version used for evaluation.
    pub rubric_version: u32,
}

/// A recorded event in the JSONL log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub(crate) enum LedgerEvent {
    ProblemObserved {
        event_id: String,
        finding: ProblemFinding,
        fingerprint: String,
        timestamp_ms: i64,
    },
    ProblemTransitioned {
        event_id: String,
        transition: ProblemTransition,
        timestamp_ms: i64,
    },
}

const PROBLEM_EVENT_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LedgerEnvelope {
    schema_version: u32,
    event: LedgerEvent,
}

/// Append-only problem ledger port.
#[async_trait]
pub trait ProblemLedger: Send + Sync {
    /// Observe a new problem finding (creates record in Observed state).
    async fn observe(&self, finding: ProblemFinding) -> Result<(), ProblemError>;

    /// Apply a state transition to a problem.
    async fn transition(&self, event: ProblemTransition) -> Result<(), ProblemError>;

    /// Get the current state of a problem by ID.
    async fn get(&self, id: &str) -> Result<Option<ProblemRecord>, ProblemError>;

    /// List all currently active (non-resolved, non-disputed) problems.
    async fn active(&self) -> Result<Vec<ProblemRecord>, ProblemError>;
}

/// JSONL-backed append-only problem ledger.
///
/// Appends one JSON event per line. On construction, replays all events
/// to rebuild current projections.
pub struct JsonlProblemLedger {
    path: PathBuf,
    projection: Mutex<Projection>,
}

impl JsonlProblemLedger {
    /// Open or create a JSONL problem ledger at the given path.
    ///
    /// If the file exists, replays all events to rebuild projections.
    /// If the file does not exist, creates a fresh empty ledger.
    pub async fn new(path: PathBuf) -> Result<Self, ProblemError> {
        let mut projection = Projection::new();

        if path.exists() {
            let mut file = File::open(&path)
                .await
                .map_err(|e| ProblemError::Persistence(format!("cannot open ledger: {e}")))?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .await
                .map_err(|e| ProblemError::Persistence(format!("cannot read ledger: {e}")))?;

            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let envelope: LedgerEnvelope = serde_json::from_str(line)
                    .map_err(|e| ProblemError::Persistence(format!("corrupt ledger line: {e}")))?;
                if envelope.schema_version != PROBLEM_EVENT_SCHEMA_V1 {
                    return Err(ProblemError::Persistence(format!(
                        "unsupported problem event schema version {}",
                        envelope.schema_version
                    )));
                }
                projection.apply_event(&envelope.event);
            }
        }

        Ok(Self {
            path,
            projection: Mutex::new(projection),
        })
    }

    async fn append_event(&self, event: &LedgerEvent) -> Result<(), ProblemError> {
        let mut line = serde_json::to_string(&LedgerEnvelope {
            schema_version: PROBLEM_EVENT_SCHEMA_V1,
            event: event.clone(),
        })
        .map_err(|e| ProblemError::Persistence(format!("serialization error: {e}")))?;
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| {
                ProblemError::Persistence(format!("cannot open ledger for append: {e}"))
            })?;

        file.write_all(line.as_bytes())
            .await
            .map_err(|e| ProblemError::Persistence(format!("cannot write to ledger: {e}")))?;

        file.flush()
            .await
            .map_err(|e| ProblemError::Persistence(format!("cannot flush ledger: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl ProblemLedger for JsonlProblemLedger {
    async fn observe(&self, finding: ProblemFinding) -> Result<(), ProblemError> {
        let mut proj = self.projection.lock().await;
        if proj.contains(&finding.problem_id) {
            return Err(ProblemError::AlreadyExists(finding.problem_id.clone()));
        }
        let fingerprint = problem_fingerprint(
            &finding.domain,
            &finding.subject,
            &finding.category,
            &finding.failure_signature,
            finding.rubric_version,
        );
        let event = LedgerEvent::ProblemObserved {
            event_id: format!("evt-{}", uuid::Uuid::new_v4()),
            fingerprint,
            timestamp_ms: finding.observed_at_ms,
            finding,
        };
        self.append_event(&event).await?;
        proj.apply_event(&event);
        Ok(())
    }

    async fn transition(&self, event: ProblemTransition) -> Result<(), ProblemError> {
        // Validate state transition first
        if !ProblemRecord::is_valid_transition(event.old_state, event.new_state) {
            return Err(ProblemError::InvalidTransition {
                from: event.old_state,
                to: event.new_state,
            });
        }

        let mut proj = self.projection.lock().await;
        let current = proj
            .get(&event.problem_id)
            .ok_or_else(|| ProblemError::NotFound(event.problem_id.clone()))?;
        if current.state != event.old_state {
            return Err(ProblemError::InvalidTransition {
                from: current.state,
                to: event.new_state,
            });
        }
        let ledger_event = LedgerEvent::ProblemTransitioned {
            event_id: event.event_id.clone(),
            timestamp_ms: event.timestamp_ms,
            transition: event,
        };

        self.append_event(&ledger_event).await?;
        proj.apply_event(&ledger_event);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<ProblemRecord>, ProblemError> {
        let proj = self.projection.lock().await;
        Ok(proj.get(id).cloned())
    }

    async fn active(&self) -> Result<Vec<ProblemRecord>, ProblemError> {
        let proj = self.projection.lock().await;
        Ok(proj.active())
    }
}
