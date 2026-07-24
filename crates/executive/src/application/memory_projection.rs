//! Policy-gated publication of Goal and approval records onto the event spine.
//!
//! Source records are persisted by their owning repositories first. This
//! boundary records an idempotent memory-candidate source event and advances
//! the deterministic memory-job reducer; it never writes a memory backend
//! directly. M05 owns extraction, approval and durable memory consolidation.

use std::sync::{Arc, Mutex};

use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventSpine, EventTreeId, EventVisibility, MessageId, NamespaceId, SchemaId, UnsequencedEvent,
};
use mnemosyne::MemorySensitivity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::application::event_projection::EventProjectionSink;
use crate::application::goal::{GoalCompletionSummary, GoalProjectionEvidence};

const MAX_DECISION_BODY_BYTES: usize = 64 * 1024;
const MEMORY_SOURCE_EVENT_NAMESPACE: Uuid = Uuid::from_u128(0xf783b8b3_4109_40cc_a8d3_191011d122c1);

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
    Queued {
        record_id: String,
        source_event_id: String,
    },
    Excluded {
        reason: &'static str,
    },
    Degraded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryProjectionHealth {
    pub degraded: bool,
    pub last_error_category: Option<&'static str>,
    pub last_record_id: Option<String>,
    pub last_source_event_id: Option<String>,
}

#[derive(Clone)]
pub struct MemoryProjection {
    event_spine: Arc<dyn EventSpine>,
    event_projections: Arc<dyn EventProjectionSink>,
    health: Arc<Mutex<MemoryProjectionHealth>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MemoryCandidateSource {
    record_id: String,
    kind: String,
    content: serde_json::Value,
    sensitivity: MemorySensitivity,
}

impl MemoryProjection {
    pub fn new(
        event_spine: Arc<dyn EventSpine>,
        event_projections: Arc<dyn EventProjectionSink>,
    ) -> Self {
        Self {
            event_spine,
            event_projections,
            health: Arc::new(Mutex::new(MemoryProjectionHealth::default())),
        }
    }

    pub fn health(&self) -> Arc<Mutex<MemoryProjectionHealth>> {
        self.health.clone()
    }

    /// Queue an immutable summary only after it has been read back from the
    /// Goal store. Stable source-event IDs make replay/restart idempotent.
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
        let content = serde_json::json!({
            "goal_id": summary.goal_id.0,
            "attempt_ids": evidence.attempt_ids,
            "artifact_ids": evidence.artifact_ids,
            "approval_id": summary.approval_id.0,
            "approval_status": summary.approval.status,
            "principal_id": summary.approval.principal_id,
            "outcome": summary.final_state,
            "verification": evidence.verification,
            "intent": summary.intent,
            "changed_files": summary.changed_files,
            "risks": summary.risks,
            "source_commit": evidence.source_commit,
            "observed_at_ms": summary.generated_at_ms,
        });
        self.queue(
            format!("goal:{}", summary.goal_id.0),
            MemoryCandidateSource {
                record_id,
                kind: "goal_outcome".into(),
                content,
                sensitivity,
            },
        )
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
        self.queue(
            format!("decision:{}", decision.decision_id),
            MemoryCandidateSource {
                record_id,
                kind: "architecture_decision".into(),
                content: serde_json::json!({
                    "decision_id": decision.decision_id,
                    "approval_id": decision.approval_id,
                    "title": decision.title,
                    "content": decision.content,
                    "principal_id": decision.principal_id,
                    "source_commit": decision.source_commit,
                    "approved_at_ms": decision.approved_at_ms,
                    "supersedes": decision.supersedes,
                }),
                sensitivity: decision.sensitivity,
            },
        )
    }

    fn queue(&self, source: String, candidate: MemoryCandidateSource) -> ProjectionStatus {
        let record_id = candidate.record_id.clone();
        let payload = match serde_json::to_value(candidate) {
            Ok(payload) => payload,
            Err(_) => return self.degraded("memory_candidate_encode_failed"),
        };
        let event_id = EventId(Uuid::new_v5(
            &MEMORY_SOURCE_EVENT_NAMESPACE,
            record_id.as_bytes(),
        ));
        let tree_id = EventTreeId::for_root_session(&source);
        let mut envelope = EnvelopeV2::new(
            SchemaId(SchemaId::EVENT_MEMORY_CANDIDATE_V1.into()),
            EnvelopeV2Target("memory-projection".into()),
            EnvelopeV2Target(format!("memory-job:{source}")),
            EnvelopeV2Delivery::Direct,
            NamespaceId(format!("memory:{source}")),
            payload.clone(),
        );
        envelope.id = MessageId(event_id.0);
        let event = match self.event_spine.append(UnsequencedEvent {
            tree_id,
            event_id,
            parent: None,
            identity: EventIdentity {
                root_session_id: source.clone(),
                session_id: source.clone(),
                agent_id: None,
            },
            envelope,
            visibility: EventVisibility::Control,
            payload: EventPayload::Inline { value: payload },
        }) {
            Ok(event) => event,
            Err(_) => return self.degraded("event_spine_append_failed"),
        };

        let report = self.event_projections.project(&event);
        if report
            .failures
            .iter()
            .any(|failure| failure.projection == "memory-jobs")
        {
            return self.degraded("memory_job_projection_failed");
        }

        let source_event_id = event.position.event_id.to_string();
        let mut health = self.health.lock().unwrap();
        health.last_record_id = Some(record_id.clone());
        health.last_source_event_id = Some(source_event_id.clone());
        ProjectionStatus::Queued {
            record_id,
            source_event_id,
        }
    }

    fn degraded(&self, category: &'static str) -> ProjectionStatus {
        let mut health = self.health.lock().unwrap();
        health.degraded = true;
        health.last_error_category = Some(category);
        ProjectionStatus::Degraded
    }
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
