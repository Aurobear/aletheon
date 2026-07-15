//! Durable, policy-gated projection of Goal outcomes and architecture decisions.
//!
//! Projection is deliberately best-effort: callers persist the source record
//! first, then invoke this boundary. Memory/spool failures are reported through
//! sanitized health and never change the persisted Goal or approval result.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use mnemosyne::{
    ExperienceEvent, MemoryMetadata, MemoryProvenance, MemorySensitivity, MemoryService,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::r#impl::goal::{GoalCompletionSummary, GoalProjectionEvidence};

const MAX_DECISION_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedArchitectureDecision {
    pub decision_id: String,
    pub approval_id: String,
    pub title: String,
    pub content: String,
    pub principal_id: String,
    pub source_commit: String,
    pub approved_at_ms: i64,
    pub supersedes: Option<String>,
    pub sensitivity: MemorySensitivity,
    /// Must be true only after the approval decision is durably persisted.
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionStatus {
    Recorded { record_id: String },
    Excluded { reason: &'static str },
    Degraded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryProjectionHealth {
    pub degraded: bool,
    pub last_error_category: Option<&'static str>,
    pub last_record_id: Option<String>,
}

#[derive(Clone)]
pub struct MemoryProjection {
    memory: Arc<dyn MemoryService>,
    health: Arc<Mutex<MemoryProjectionHealth>>,
}

impl MemoryProjection {
    pub fn new(memory: Arc<dyn MemoryService>) -> Self {
        Self {
            memory,
            health: Arc::new(Mutex::new(MemoryProjectionHealth::default())),
        }
    }

    pub fn health(&self) -> Arc<Mutex<MemoryProjectionHealth>> {
        self.health.clone()
    }

    /// Project an immutable summary only after it has been read back from the
    /// Goal store. Deterministic IDs make replay/restart idempotent downstream.
    pub async fn project_goal_summary(
        &self,
        summary: &GoalCompletionSummary,
        evidence: &GoalProjectionEvidence,
        sensitivity: MemorySensitivity,
    ) -> ProjectionStatus {
        if !matches!(
            summary.final_state.as_str(),
            "completed" | "failed" | "cancelled"
        ) && !matches!(summary.approval.status.as_str(), "approved" | "rejected")
        {
            return ProjectionStatus::Excluded {
                reason: "summary is neither terminal nor approval-resolved",
            };
        }
        if is_excluded(&sensitivity) {
            return ProjectionStatus::Excluded {
                reason: "sensitive outcome",
            };
        }
        let record_id = format!(
            "goal:{}:approval:{}:outcome",
            summary.goal_id.0, summary.approval_id.0
        );
        let observed = timestamp(summary.generated_at_ms);
        let content = serde_json::json!({
            "goal_id": summary.goal_id.0,
            "attempt_ids": evidence.attempt_ids,
            "artifact_ids": evidence.artifact_ids,
            "approval_id": summary.approval_id.0,
            "approval_status": summary.approval.status,
            "outcome": summary.final_state,
            "verification": evidence.verification,
            "intent": summary.intent,
            "changed_files": summary.changed_files,
            "risks": summary.risks,
        })
        .to_string();
        let event = ExperienceEvent::GoalOutcome {
            goal_id: summary.goal_id.0.to_string(),
            outcome: summary.final_state.clone(),
            content,
            metadata: metadata(
                record_id.clone(),
                format!("goal-summary:{}", summary.approval_id.0),
                summary.approval.principal_id.clone(),
                evidence.source_commit.clone(),
                observed,
                None,
                sensitivity,
            ),
        };
        self.record(event, record_id).await
    }

    pub async fn project_architecture_decision(
        &self,
        decision: &ApprovedArchitectureDecision,
    ) -> ProjectionStatus {
        if !decision.approved {
            return ProjectionStatus::Excluded {
                reason: "architecture decision is not approved",
            };
        }
        if is_excluded(&decision.sensitivity) {
            return ProjectionStatus::Excluded {
                reason: "sensitive decision",
            };
        }
        if decision.content.len() > MAX_DECISION_BODY_BYTES {
            return ProjectionStatus::Excluded {
                reason: "architecture decision exceeds byte limit",
            };
        }
        let record_id = format!(
            "decision:{}:{}",
            decision.decision_id,
            short_hash(&decision.approval_id)
        );
        let event = ExperienceEvent::ArchitectureDecision {
            title: decision.title.clone(),
            content: decision.content.clone(),
            metadata: metadata(
                record_id.clone(),
                format!("architecture-decision:{}", decision.decision_id),
                Some(decision.principal_id.clone()),
                Some(decision.source_commit.clone()),
                timestamp(decision.approved_at_ms),
                decision.supersedes.clone(),
                decision.sensitivity.clone(),
            ),
        };
        self.record(event, record_id).await
    }

    async fn record(&self, event: ExperienceEvent, record_id: String) -> ProjectionStatus {
        match self.memory.record(event).await {
            Ok(()) => {
                let mut health = self.health.lock().unwrap();
                health.last_record_id = Some(record_id.clone());
                ProjectionStatus::Recorded { record_id }
            }
            Err(_) => {
                let mut health = self.health.lock().unwrap();
                health.degraded = true;
                health.last_error_category = Some("memory_record_failed");
                ProjectionStatus::Degraded
            }
        }
    }
}

fn metadata(
    record_id: String,
    source_id: String,
    principal: Option<String>,
    source_commit: Option<String>,
    observed_time: DateTime<Utc>,
    supersedes: Option<String>,
    sensitivity: MemorySensitivity,
) -> MemoryMetadata {
    MemoryMetadata {
        record_id,
        provenance: MemoryProvenance {
            source: "aletheon".into(),
            source_id,
            principal,
            source_commit,
        },
        source_time: Some(observed_time),
        observed_time,
        valid_from: Some(observed_time),
        valid_until: None,
        supersedes,
        superseded_by: None,
        confidence: 1.0,
        sensitivity,
    }
}

fn timestamp(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn is_excluded(sensitivity: &MemorySensitivity) -> bool {
    matches!(
        sensitivity,
        MemorySensitivity::Confidential | MemorySensitivity::Restricted
    )
}

fn short_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..16].to_string()
}
