//! Policy-gated publication of Goal and approval records onto the event spine.
//!
//! Source records are persisted by their owning repositories first. This
//! boundary records an idempotent memory-candidate source event and advances
//! the deterministic memory-job reducer; it never writes a memory backend
//! directly. M05 owns extraction, approval and durable memory consolidation.

use std::sync::{Arc, Mutex};

use ::contracts::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, MessageId, NamespaceId, SchemaId,
};
pub use mnemosyne::candidate_projection::ApprovedArchitectureDecision;
use mnemosyne::candidate_projection::{
    architecture_decision_candidate, candidate_sensitivity_allowed, MemoryCandidateSource,
};
use mnemosyne::MemorySensitivity;
use runtime::{
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility,
    UnsequencedEvent,
};
use uuid::Uuid;

use adapters_sqlite::goal::GoalCompletionSummary;
use application::goal_projection::GoalProjectionEvidence;
use runtime::read_model::EventProjectionSink;

const MEMORY_SOURCE_EVENT_NAMESPACE: Uuid = Uuid::from_u128(0xf783b8b3_4109_40cc_a8d3_191011d122c1);

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

#[derive(Clone)]
pub struct MemoryEvaluationProjectionSink {
    projection: MemoryProjection,
}

impl MemoryEvaluationProjectionSink {
    pub fn new(projection: MemoryProjection) -> Self {
        Self { projection }
    }
}

#[async_trait::async_trait]
impl application::evaluation_projection::EvaluationProjectionSink
    for MemoryEvaluationProjectionSink
{
    fn name(&self) -> &'static str {
        "mnemosyne"
    }

    async fn project(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> anyhow::Result<()> {
        match self.projection.project_evaluation_receipt(record) {
            ProjectionStatus::Queued { .. } | ProjectionStatus::Excluded { .. } => Ok(()),
            ProjectionStatus::Degraded => anyhow::bail!("memory evaluation projection degraded"),
        }
    }
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

    /// Queue an evidence-linked experience from the persisted receipt only.
    /// The memory reducer decides later whether it is consolidated.
    pub fn project_evaluation_receipt(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> ProjectionStatus {
        let receipt_id = record.receipt.receipt_id.0.to_string();
        self.queue(
            format!("evaluation:{receipt_id}"),
            MemoryCandidateSource::new(
                format!("evaluation:{receipt_id}"),
                "evaluation_outcome",
                serde_json::json!({
                    "receipt_ref": record.receipt,
                    "evidence_ref": format!("evaluation-receipt:{receipt_id}"),
                    "runtime_id": record.context.runtime_id,
                    "profile_id": record.context.profile_id,
                    "rubric_id": record.context.rubric_id,
                    "rubric_version": record.context.rubric_version,
                }),
                MemorySensitivity::Internal,
            ),
        )
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
        if !candidate_sensitivity_allowed(&sensitivity) {
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
            MemoryCandidateSource::new(record_id, "goal_outcome", content, sensitivity),
        )
    }

    pub async fn project_architecture_decision(
        &self,
        decision: &ApprovedArchitectureDecision,
    ) -> ProjectionStatus {
        let candidate = match architecture_decision_candidate(decision) {
            Ok(candidate) => candidate,
            Err(reason) => return ProjectionStatus::Excluded { reason },
        };
        self.queue(format!("decision:{}", decision.decision_id), candidate)
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
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        health.last_record_id = Some(record_id.clone());
        health.last_source_event_id = Some(source_event_id.clone());
        ProjectionStatus::Queued {
            record_id,
            source_event_id,
        }
    }

    fn degraded(&self, category: &'static str) -> ProjectionStatus {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        health.degraded = true;
        health.last_error_category = Some(category);
        ProjectionStatus::Degraded
    }
}
