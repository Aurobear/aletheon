//! Bounded, redacted audit summary for an approval-driven Goal outcome.

use super::ObjectiveStore;
use crate::application::approval::ApprovalApplyReceipt;
use anyhow::{bail, Context, Result};
use fabric::{ApprovalId, ApprovalSnapshot, GoalId};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ATTEMPTS: usize = 20;
const MAX_CHANGED_FILES: usize = 512;
const MAX_CHECKS: usize = 128;
const MAX_RISKS: usize = 128;
const MAX_TEXT_BYTES: usize = 1024;
const MAX_INTENT_BYTES: usize = 4096;
const MAX_SUMMARY_JSON_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalAttemptOutcomeSummary {
    pub sequence: u32,
    pub runtime_id: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCheckOutcomeSummary {
    pub name: String,
    pub passed: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalApprovalOutcomeSummary {
    pub status: String,
    pub principal_id: Option<String>,
    pub channel: Option<String>,
    pub reason: Option<String>,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalApplyOutcomeSummary {
    pub operation_id: String,
    pub success: bool,
    pub applied_head: Option<String>,
    pub diff_sha256: String,
    pub changed_paths: Vec<PathBuf>,
    pub error: Option<String>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCompletionSummary {
    pub goal_id: GoalId,
    pub approval_id: ApprovalId,
    pub intent: String,
    pub attempts: Vec<GoalAttemptOutcomeSummary>,
    pub changed_files: Vec<PathBuf>,
    pub checks: Vec<GoalCheckOutcomeSummary>,
    pub approval: GoalApprovalOutcomeSummary,
    pub apply: Option<GoalApplyOutcomeSummary>,
    pub risks: Vec<String>,
    pub final_state: String,
    pub generated_at_ms: i64,
}

impl GoalCompletionSummary {
    pub fn build(
        store: &ObjectiveStore,
        approval: &ApprovalSnapshot,
        receipt: Option<&ApprovalApplyReceipt>,
        now_ms: i64,
    ) -> Result<Self> {
        let goal = store
            .get_goal(approval.subject.goal_id)?
            .context("Goal missing while building completion summary")?;
        let attempts = store
            .attempts_for_goal(goal.id, MAX_ATTEMPTS)?
            .into_iter()
            .map(|attempt| {
                let summary = attempt
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.as_str())
                    .or_else(|| attempt.output.as_ref().map(|output| output.output.as_str()))
                    .unwrap_or("no terminal output");
                GoalAttemptOutcomeSummary {
                    sequence: attempt.sequence,
                    runtime_id: sanitize(&attempt.runtime_id.0, MAX_TEXT_BYTES),
                    status: wire(&attempt.status),
                    summary: sanitize(summary, MAX_TEXT_BYTES),
                }
            })
            .collect();

        let mut changed_files = Vec::new();
        let mut checks = Vec::new();
        let mut risks = Vec::new();
        if let Some(job_id) = approval.subject.job_id {
            if let Some(coding) = store.load_coding_job(job_id)? {
                changed_files = coding
                    .report
                    .changed_files
                    .into_iter()
                    .take(MAX_CHANGED_FILES)
                    .map(|file| file.path)
                    .collect();
            }
            if let Some(verification) = store.load_verification_report(job_id)? {
                checks = verification
                    .report
                    .checks
                    .into_iter()
                    .take(MAX_CHECKS)
                    .map(|check| GoalCheckOutcomeSummary {
                        name: sanitize(&check.name, MAX_TEXT_BYTES),
                        passed: check.passed,
                        timed_out: check.timed_out,
                        cancelled: check.cancelled,
                        summary: sanitize(&check.summary, MAX_TEXT_BYTES),
                    })
                    .collect();
                risks = verification
                    .report
                    .risk_summary
                    .into_iter()
                    .take(MAX_RISKS)
                    .map(|risk| sanitize(&risk, MAX_TEXT_BYTES))
                    .collect();
            }
        }
        let resolution = approval.resolution.as_ref();
        let apply = receipt.map(|receipt| GoalApplyOutcomeSummary {
            operation_id: receipt.operation_id.0.to_string(),
            success: receipt.success,
            applied_head: receipt
                .applied_head
                .as_deref()
                .map(|value| sanitize(value, MAX_TEXT_BYTES)),
            diff_sha256: sanitize(&receipt.diff_sha256, MAX_TEXT_BYTES),
            changed_paths: receipt
                .changed_paths
                .iter()
                .take(MAX_CHANGED_FILES)
                .cloned()
                .collect(),
            error: receipt
                .error
                .as_deref()
                .map(|value| sanitize(value, MAX_TEXT_BYTES)),
            finished_at_ms: receipt.finished_at_ms,
        });
        let summary = Self {
            goal_id: goal.id,
            approval_id: approval.id,
            intent: sanitize(&goal.spec.original_intent, MAX_INTENT_BYTES),
            attempts,
            changed_files,
            checks,
            approval: GoalApprovalOutcomeSummary {
                status: wire(&approval.status),
                principal_id: resolution
                    .and_then(|value| value.principal_id.as_ref())
                    .map(|value| sanitize(&value.0, MAX_TEXT_BYTES)),
                channel: resolution
                    .and_then(|value| value.channel.as_deref())
                    .map(|value| sanitize(value, MAX_TEXT_BYTES)),
                reason: resolution
                    .and_then(|value| value.reason.as_deref())
                    .map(|value| sanitize(value, MAX_TEXT_BYTES)),
                resolved_at_ms: resolution.map(|value| value.resolved_at_ms),
            },
            apply,
            risks,
            final_state: goal.state.as_str().into(),
            generated_at_ms: now_ms,
        };
        let encoded = serde_json::to_vec(&summary)?;
        if encoded.len() > MAX_SUMMARY_JSON_BYTES {
            bail!("Goal completion summary exceeds bounded persistence limit");
        }
        Ok(summary)
    }
}

impl ObjectiveStore {
    /// Persist before any caller emits the completion notification. Replays
    /// return the original immutable summary rather than emitting a new view.
    pub fn persist_goal_completion_summary(
        &self,
        summary: &GoalCompletionSummary,
    ) -> Result<GoalCompletionSummary> {
        if let Some(existing) = self.load_goal_completion_summary(summary.approval_id)? {
            return Ok(existing);
        }
        let json = serde_json::to_string(summary)?;
        if json.len() > MAX_SUMMARY_JSON_BYTES {
            bail!("Goal completion summary exceeds bounded persistence limit");
        }
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO goal_completion_summaries
             (approval_id,objective_id,summary_json,created_at_ms) VALUES (?1,?2,?3,?4)",
            params![
                summary.approval_id.0.to_string(),
                summary.goal_id.0,
                json,
                summary.generated_at_ms,
            ],
        )?;
        let version: u64 = tx.query_row(
            "SELECT version FROM objectives WHERE objective_id=?1",
            params![summary.goal_id.0],
            |row| row.get(0),
        )?;
        let next = version.saturating_add(1);
        tx.execute(
            "UPDATE objectives SET version=?1,updated_at=datetime('now')
             WHERE objective_id=?2 AND version=?3",
            params![next, summary.goal_id.0, version],
        )?;
        tx.execute(
            "INSERT INTO goal_events (objective_id,version,event_type,payload_json)
             VALUES (?1,?2,'completion_summary_persisted',?3)",
            params![
                summary.goal_id.0,
                next,
                serde_json::json!({"approval_id":summary.approval_id.0}).to_string(),
            ],
        )?;
        tx.commit()?;
        self.load_goal_completion_summary(summary.approval_id)?
            .context("Goal completion summary disappeared after persistence")
    }

    pub fn load_goal_completion_summary(
        &self,
        approval_id: ApprovalId,
    ) -> Result<Option<GoalCompletionSummary>> {
        self.db
            .query_row(
                "SELECT summary_json FROM goal_completion_summaries WHERE approval_id=?1",
                params![approval_id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }
}

fn sanitize(value: &str, max_bytes: usize) -> String {
    fabric::RuntimeFailure {
        class: fabric::FailureClass::ProviderPermanent,
        message: value.into(),
        retryable: false,
        usage: Default::default(),
        evidence: vec![],
    }
    .bounded_for_persistence(max_bytes)
    .message
}

fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}
