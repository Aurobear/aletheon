//! Versioned, transport-neutral session history contracts.

use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AuditEventId, OperationId, PermitId, PrincipalId, SessionId, TurnStop};

pub const SESSION_SCHEMA_VERSION: u16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TurnId(pub Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ItemId(pub Uuid);

impl ItemId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ItemId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionRecord {
    pub schema_version: u16,
    #[schemars(with = "String")]
    pub id: SessionId,
    pub parent: Option<SessionFork>,
    pub created_at_ms: u64,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionFork {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub through_sequence: u64,
}

/// Canonical control-event payload for a forked public Session view.
/// Item identities are allocated before append so replay is byte-stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionForkedEvent {
    #[schemars(with = "String")]
    pub parent_session_id: SessionId,
    pub through_sequence: u64,
    pub child: SessionRecord,
    pub inherited_items: Vec<ItemRecord>,
}

/// Canonical control event binding an authenticated principal to a Session.
/// Rebinding to a different principal is rejected by the materialized store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPrincipalBoundEvent {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    #[schemars(with = "String")]
    pub principal: PrincipalId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Interrupted,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnRecord {
    pub schema_version: u16,
    pub id: TurnId,
    #[schemars(with = "String")]
    pub session_id: SessionId,
    #[schemars(with = "Uuid")]
    pub operation_id: OperationId,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub stop: Option<TurnStop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemRecord {
    pub schema_version: u16,
    pub id: ItemId,
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub sequence: u64,
    pub created_at_ms: u64,
    pub payload: ItemPayload,
}

pub const TASK_PROJECTION_FACT_SCHEMA_VERSION: u16 = 1;

/// Host-authored durable Task read-model inputs. These facts are replayed with
/// Session items so restart recovery never depends on process-local UI state or
/// model self-reporting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskProjectionFact {
    pub schema_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<u64>,
    #[serde(default)]
    pub active_runtime_children: Vec<String>,
    #[serde(default)]
    pub active_commands: Vec<String>,
    #[serde(default)]
    pub pending_approvals: Vec<String>,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_head: Option<String>,
}

impl Default for TaskProjectionFact {
    fn default() -> Self {
        Self {
            schema_version: TASK_PROJECTION_FACT_SCHEMA_VERSION,
            plan_revision: None,
            active_runtime_children: Vec::new(),
            active_commands: Vec::new(),
            pending_approvals: Vec::new(),
            budget: None,
            checkpoint_head: None,
        }
    }
}

/// Host classification written when startup recovery finds a turn that has a
/// durable start boundary but no terminal fact. This is a Session journal fact,
/// not a process-local recovery marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnRecoveryClassification {
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ItemPayload {
    /// Authoritative per-turn start fact. The typed target is stored on the
    /// existing durable start boundary so replay cannot inherit a prior turn's
    /// target. Legacy items decode as General.
    UserMessage {
        content: String,
        #[serde(default)]
        execution_target: crate::ExecutionTargetSelection,
    },
    AssistantMessage {
        content: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
        #[schemars(with = "Option<Uuid>")]
        permit_id: Option<PermitId>,
        #[schemars(with = "Option<Uuid>")]
        audit_id: Option<AuditEventId>,
    },
    CapabilityReceipt {
        receipt: crate::CapabilityTerminalReceipt,
    },
    /// Immutable Robot-domain terminal receipt. The report contains only
    /// bounded typed facts and external artifact references; image/log bytes are
    /// never embedded in the Session journal.
    RobotEpisodeReceipt {
        #[schemars(with = "serde_json::Value")]
        receipt: Box<crate::types::episode_report::SettledEpisodeReport>,
    },
    EvaluationReceiptRef {
        receipt: crate::EvaluationReceiptRef,
    },
    ModelContextProjection {
        receipt: crate::model_projection::ModelContextProjectionReceipt,
    },
    /// Host-authoritative model/profile/history/rollout budget truth for this
    /// turn. It is computed before inference from the selected model and the
    /// exact prepared context.
    ContextBudgetProjection {
        projection: Box<crate::ContextBudgetProjection>,
    },
    /// Evidence for an applied history rewrite, including its triggering budget.
    ContextCompactionProjection {
        projection: Box<crate::ContextCompactionProjection>,
    },
    InferenceReceipt {
        receipt: crate::types::inference_receipt::InferenceTerminalReceipt,
    },
    TaskProjection {
        fact: TaskProjectionFact,
    },
    TurnRecovery {
        classification: TurnRecoveryClassification,
    },
    /// Authoritative terminal fact for an ordinarily executed turn. This is
    /// distinct from `TurnRecovery`, which is reserved for startup recovery of
    /// a turn whose terminal boundary was missing.
    TurnSettlement {
        status: crate::TurnTerminalStatus,
        content: String,
    },
    ContextProjection {
        space: String,
        broadcast_epoch: Option<u64>,
        workspace_version: Option<u64>,
        dasein_version: u64,
        content_ids: Vec<String>,
    },
    SystemNotice {
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SessionNotification {
    ItemAppended {
        schema_version: u16,
        item: ItemRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SessionProtocolV5 {
    Session(SessionRecord),
    Turn(TurnRecord),
    Item(ItemRecord),
    Notification(SessionNotification),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EvaluationContractId, EvaluationDecision, EvaluationReceiptId, EVALUATION_SCHEMA_V1,
    };

    #[test]
    fn receipt_ref_round_trips_in_session_item() {
        let payload = ItemPayload::EvaluationReceiptRef {
            receipt: crate::EvaluationReceiptRef {
                schema_version: EVALUATION_SCHEMA_V1,
                receipt_id: EvaluationReceiptId::new(),
                contract_id: EvaluationContractId::new(),
                subject_kind: "turn".into(),
                subject_id: TurnId::new().0.to_string(),
                decision: EvaluationDecision::ObservedPass,
                weighted_total_millis: Some(82_000),
                evidence_coverage_millis: 900,
                confidence_millis: 850,
                failed_gates: vec![],
                created_at_ms: 42,
            },
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert_eq!(serde_json::from_str::<ItemPayload>(&json).unwrap(), payload);
    }

    #[test]
    fn ordinary_turn_settlement_is_distinct_from_recovery() {
        let payload = ItemPayload::TurnSettlement {
            status: crate::TurnTerminalStatus::Interrupted,
            content: "Cancelled by user".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("turn_settlement"));
        assert!(!json.contains("turn_recovery"));
        assert_eq!(serde_json::from_str::<ItemPayload>(&json).unwrap(), payload);
    }

    #[test]
    fn current_payload_decoder_preserves_prior_tagged_items() {
        let json = r#"{"type":"assistant_message","data":{"content":"done"}}"#;
        assert_eq!(
            serde_json::from_str::<ItemPayload>(json).unwrap(),
            ItemPayload::AssistantMessage {
                content: "done".into()
            }
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppendOutcome {
    Appended,
    AlreadyPresent,
}

/// Read side of the canonical Session authority contract. Projection stores may
/// implement this trait without gaining authority to originate Session facts.
#[async_trait]
pub trait SessionReadStore: Send + Sync {
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>>;
    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>>;
    /// Read a bounded, sequence-ordered page after `after`. Stores with a
    /// native LIMIT implementation should override this default so hot paths
    /// never materialize the complete session merely to truncate it.
    async fn load_items_page(
        &self,
        session: &SessionId,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ItemRecord>> {
        let mut items = self.load_items(session, after).await?;
        items.truncate(limit.max(1));
        Ok(items)
    }
    async fn list_sessions(&self, _limit: usize) -> Result<Vec<SessionRecord>> {
        anyhow::bail!("Session listing is unavailable for this store")
    }

    /// List session records together with their durable owner binding. Stores
    /// should override this when both projections can be read in one query.
    async fn list_sessions_with_principal(
        &self,
        limit: usize,
    ) -> Result<Vec<(SessionRecord, Option<PrincipalId>)>> {
        let mut sessions = Vec::new();
        for session in self.list_sessions(limit).await? {
            let principal = self.principal_for(&session.id).await?;
            sessions.push((session, principal));
        }
        Ok(sessions)
    }

    /// Enumerate every durable Session identity for startup reconciliation.
    async fn list_session_ids(&self) -> Result<Vec<SessionId>> {
        anyhow::bail!("Session identity enumeration is unavailable for this store")
    }

    /// Read the durable principal binding used by picker/snapshot isolation.
    async fn principal_for(&self, _session: &SessionId) -> Result<Option<PrincipalId>> {
        Ok(None)
    }
}

/// The single authoritative Session mutation port. Production implementations
/// must commit through the EventSpine before updating any materialized read
/// model; read-only projection stores implement `SessionReadStore` instead.
#[async_trait]
pub trait SessionAppendStore: SessionReadStore {
    async fn create(&self, session: SessionRecord) -> Result<()>;
    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome>;
    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()>;

    /// Bind a durable authenticated principal to a session. Implementations
    /// must reject rebinding to a different principal.
    async fn bind_principal(&self, session: &SessionId, principal: &PrincipalId) -> Result<()>;
}
