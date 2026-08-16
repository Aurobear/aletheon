//! Runtime-owned settlement evidence contract and EventSpine adapter.

use crate::{
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility,
    UnsequencedEvent,
};
use ::contracts::{
    AgentControlError, AgentResourceClass, EnvelopeV2, NamespaceId, OperationId, ReparentReceipt,
    SettlementPhase,
};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementEvidence {
    Phase(SettlementPhase),
    Reparented(ReparentReceipt),
    ResourceTerminated {
        resource_id: String,
        class: AgentResourceClass,
        reason: String,
    },
    IdempotentReplay {
        idempotency_key: String,
    },
    GenerationRejected(String, String),
}

#[async_trait]
pub trait SettlementEvidenceSink: Send + Sync {
    async fn record(&self, evidence: SettlementEvidence) -> Result<(), AgentControlError>;
}

#[derive(Debug, Default)]
pub struct NoopSettlementEvidenceSink;

#[async_trait]
impl SettlementEvidenceSink for NoopSettlementEvidenceSink {
    async fn record(&self, _evidence: SettlementEvidence) -> Result<(), AgentControlError> {
        Ok(())
    }
}

/// EventSpine-backed evidence adapter. Runtime owns the event schema and
/// ordering; the host settlement engine only submits typed evidence.
pub struct SpineSettlementEvidenceSink {
    spine: Arc<dyn EventSpine>,
    root_agent_id: String,
    agent_id: String,
    operation_id: OperationId,
}

impl SpineSettlementEvidenceSink {
    pub fn new(
        spine: Arc<dyn EventSpine>,
        root_agent_id: String,
        agent_id: String,
        operation_id: OperationId,
    ) -> Self {
        Self {
            spine,
            root_agent_id,
            agent_id,
            operation_id,
        }
    }
}

#[async_trait]
impl SettlementEvidenceSink for SpineSettlementEvidenceSink {
    async fn record(&self, evidence: SettlementEvidence) -> Result<(), AgentControlError> {
        let (kind, detail) = match evidence {
            SettlementEvidence::Phase(phase) => (
                "agent.settlement.phase",
                serde_json::json!({"phase": format!("{phase:?}")}),
            ),
            SettlementEvidence::Reparented(receipt) => (
                "agent.reparent",
                serde_json::to_value(receipt).map_err(persistence)?,
            ),
            SettlementEvidence::ResourceTerminated {
                resource_id,
                class,
                reason,
            } => (
                "agent.orphan_killed",
                serde_json::json!({"resource_id": resource_id, "class": class, "reason": reason}),
            ),
            SettlementEvidence::IdempotentReplay { idempotency_key } => (
                "agent.settlement.replay",
                serde_json::json!({"idempotency_key": idempotency_key}),
            ),
            SettlementEvidence::GenerationRejected(expected, received) => (
                "agent.settlement.generation_rejected",
                serde_json::json!({"expected": expected, "received": received}),
            ),
        };
        let payload = serde_json::json!({
            "kind": kind,
            "agent_id": self.agent_id,
            "detail": detail,
        });
        let mut envelope = EnvelopeV2::new(
            ::contracts::SchemaId::from(::contracts::SchemaId::EVENT_AGENT_SETTLEMENT_V1),
            ::contracts::EnvelopeV2Target(format!("agent:{}", self.agent_id)),
            ::contracts::EnvelopeV2Target(format!("agent-tree:{}", self.root_agent_id)),
            ::contracts::EnvelopeV2Delivery::FanOut,
            NamespaceId(format!("agent-tree:{}", self.root_agent_id)),
            payload.clone(),
        );
        envelope = envelope.with_operation_id(self.operation_id);
        self.spine
            .append(UnsequencedEvent {
                tree_id: EventTreeId::for_root_session(&self.root_agent_id),
                event_id: EventId::new(),
                parent: None,
                identity: EventIdentity {
                    root_session_id: self.root_agent_id.clone(),
                    session_id: self.root_agent_id.clone(),
                    agent_id: Some(self.agent_id.clone()),
                },
                envelope,
                visibility: EventVisibility::Control,
                payload: EventPayload::Inline { value: payload },
            })
            .map(|_| ())
            .map_err(persistence)
    }
}

fn persistence(error: impl std::fmt::Display) -> AgentControlError {
    AgentControlError {
        kind: ::contracts::AgentControlErrorKind::Persistence,
        message: error.to_string(),
    }
}
