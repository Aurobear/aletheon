//! Explicit child-agent resource settlement state machine.
//!
//! The engine deliberately depends on narrow ports. Resource ownership changes,
//! receipt durability, and lease deletion remain implemented by their owning
//! subsystems, while this module provides ordering, policy, and replay safety.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    can_reparent, settlement_idempotency_key, AgentControlError, AgentControlErrorKind,
    AgentRecoveryDecision, AgentResourceClass, BackgroundResourceDecl, EnvelopeV2, EventId,
    EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility, NamespaceId,
    OperationId, ReparentContext, ReparentReceipt, SettlementPhase, SettlementReceipt,
    SettlementTerminal, UnsequencedEvent,
};
use rusqlite::OptionalExtension;
use tokio::sync::Mutex;

use super::{AgentAdmissionLease, AgentRunRepository, LiveAgentRun};

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
        };
        let payload = serde_json::json!({
            "kind": kind,
            "agent_id": self.agent_id,
            "detail": detail,
        });
        let mut envelope = EnvelopeV2::new(
            fabric::SchemaId::from(fabric::SchemaId::TURN_EVENT_V1),
            fabric::EnvelopeV2Target(format!("agent:{}", self.agent_id)),
            fabric::EnvelopeV2Target(format!("agent-tree:{}", self.root_agent_id)),
            fabric::EnvelopeV2Delivery::FanOut,
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

/// Durable implementations must use `receipt.idempotency_key` as a unique key.
#[async_trait]
pub trait SettlementReceiptStore: Send + Sync {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError>;

    /// Store the receipt if absent and return the authoritative receipt. This
    /// makes competing attempts converge on one immutable result.
    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError>;
}

#[derive(Debug, Default)]
pub struct InMemorySettlementReceiptStore {
    receipts: Mutex<HashMap<String, SettlementReceipt>>,
}

pub struct SqliteSettlementReceiptStore {
    connection: parking_lot::Mutex<rusqlite::Connection>,
}

impl SqliteSettlementReceiptStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, AgentControlError> {
        let connection = rusqlite::Connection::open(path).map_err(persistence)?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS agent_settlement_receipts (
                    idempotency_key TEXT PRIMARY KEY NOT NULL,
                    receipt_json TEXT NOT NULL
                );",
            )
            .map_err(persistence)?;
        Ok(Self {
            connection: parking_lot::Mutex::new(connection),
        })
    }
}

#[async_trait]
impl SettlementReceiptStore for SqliteSettlementReceiptStore {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError> {
        let connection = self.connection.lock();
        let encoded = connection
            .query_row(
                "SELECT receipt_json FROM agent_settlement_receipts WHERE idempotency_key=?1",
                rusqlite::params![idempotency_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(persistence)?;
        encoded
            .map(|value| serde_json::from_str(&value).map_err(persistence))
            .transpose()
    }

    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError> {
        let encoded = serde_json::to_string(&receipt).map_err(persistence)?;
        let connection = self.connection.lock();
        connection
            .execute(
                "INSERT OR IGNORE INTO agent_settlement_receipts(idempotency_key,receipt_json)
                 VALUES (?1,?2)",
                rusqlite::params![&receipt.idempotency_key, encoded],
            )
            .map_err(persistence)?;
        let authoritative = connection
            .query_row(
                "SELECT receipt_json FROM agent_settlement_receipts WHERE idempotency_key=?1",
                rusqlite::params![&receipt.idempotency_key],
                |row| row.get::<_, String>(0),
            )
            .map_err(persistence)?;
        serde_json::from_str(&authoritative).map_err(persistence)
    }
}

pub struct RepositorySettlementLeasePort {
    repository: Arc<dyn AgentRunRepository>,
}

impl RepositorySettlementLeasePort {
    pub fn new(repository: Arc<dyn AgentRunRepository>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl SettlementLeasePort for RepositorySettlementLeasePort {
    async fn release(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError> {
        self.repository
            .delete_resource_lease(lease_key, expected_owner)
            .await
    }
}

/// Safe production default until a managed background-command backend is
/// installed. It never grants reparent and cancels the child scope before
/// acknowledging disposal, so declarations cannot create orphans.
pub struct FailClosedSettlementResourcePort {
    cancellation: tokio_util::sync::CancellationToken,
}

/// Production resource backend backed by the resource controls fixed in the
/// live run at spawn time. Operations are independently cancellable and owner
/// transitions are action-key idempotent.
pub struct ManagedSettlementResourcePort {
    live: LiveAgentRun,
    parent_authority_covers: bool,
    parent_budget_accepts: ParentBudgetAcceptance,
    parent_cancellation: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Clone, Default)]
struct ParentBudgetAcceptance(Arc<AtomicBool>);

impl ParentBudgetAcceptance {
    fn publish(&self, accepted: bool) {
        self.0.store(accepted, Ordering::Release);
    }

    fn read(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl ManagedSettlementResourcePort {
    pub fn new(
        live: LiveAgentRun,
        parent_authority_covers: bool,
        parent_budget_accepts: bool,
        parent_cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Self {
        Self {
            live,
            parent_authority_covers,
            parent_budget_accepts: {
                let gate = ParentBudgetAcceptance::default();
                gate.publish(parent_budget_accepts);
                gate
            },
            parent_cancellation,
        }
    }

    /// Publish the authoritative transfer outcome only after quiescing has
    /// closed admission and fixed the resource set.
    pub fn set_parent_budget_accepts(&self, accepted: bool) {
        self.parent_budget_accepts.publish(accepted);
    }
}

#[async_trait]
impl SettlementResourcePort for ManagedSettlementResourcePort {
    fn reparent_context(
        &self,
        resource: &BackgroundResourceDecl,
        parent_owner: &str,
    ) -> ReparentContext {
        ReparentContext {
            parent_authority_covers: self.parent_authority_covers
                && self.live.has_managed_resource(&resource.resource_id),
            parent_budget_accepts: self.parent_budget_accepts.read(),
            notification_route_transferable: !parent_owner.trim().is_empty(),
        }
    }

    async fn settle_foreground(
        &self,
        resource: &BackgroundResourceDecl,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        if self
            .live
            .terminate_managed_resource(&resource.resource_id, action_key)
            .await
        {
            Ok(())
        } else {
            Err(invalid("managed foreground resource is unavailable"))
        }
    }

    async fn terminate(
        &self,
        resource: &BackgroundResourceDecl,
        _reason: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        if self
            .live
            .terminate_managed_resource(&resource.resource_id, action_key)
            .await
        {
            Ok(())
        } else {
            Err(invalid("managed settlement resource is unavailable"))
        }
    }

    async fn reparent(
        &self,
        resource: &BackgroundResourceDecl,
        old_owner: &str,
        new_owner: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.live
            .reparent_managed_resource(
                &resource.resource_id,
                old_owner,
                new_owner,
                action_key,
                self.parent_cancellation.as_ref(),
            )
            .await
    }
}

impl FailClosedSettlementResourcePort {
    pub fn new(cancellation: tokio_util::sync::CancellationToken) -> Self {
        Self { cancellation }
    }
}

#[async_trait]
impl SettlementResourcePort for FailClosedSettlementResourcePort {
    fn reparent_context(
        &self,
        _resource: &BackgroundResourceDecl,
        _parent_owner: &str,
    ) -> ReparentContext {
        ReparentContext {
            parent_authority_covers: false,
            parent_budget_accepts: false,
            notification_route_transferable: false,
        }
    }

    async fn settle_foreground(
        &self,
        _resource: &BackgroundResourceDecl,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.cancellation.cancel();
        Ok(())
    }

    async fn terminate(
        &self,
        _resource: &BackgroundResourceDecl,
        _reason: &str,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.cancellation.cancel();
        Ok(())
    }

    async fn reparent(
        &self,
        _resource: &BackgroundResourceDecl,
        _old_owner: &str,
        _new_owner: &str,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        Err(invalid("managed resource reparent backend is unavailable"))
    }
}

pub async fn settle_admission(
    admission: &mut dyn AgentAdmissionLease,
    terminal: &SettlementTerminal,
    usage: Option<&fabric::AttemptUsage>,
) -> Result<(), AgentControlError> {
    match (terminal, usage) {
        (SettlementTerminal::Completed, Some(usage)) => admission.settle(usage).await,
        _ => admission.revoke().await,
    }
}

pub fn terminal_with_memory_flush(
    terminal: SettlementTerminal,
    memory_error: Option<AgentControlError>,
) -> SettlementTerminal {
    match memory_error {
        Some(error) => SettlementTerminal::Failed {
            reason: format!("child memory flush failed: {}", error.message),
        },
        None => terminal,
    }
}

#[async_trait]
impl SettlementReceiptStore for InMemorySettlementReceiptStore {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError> {
        Ok(self.receipts.lock().await.get(idempotency_key).cloned())
    }

    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError> {
        let mut receipts = self.receipts.lock().await;
        Ok(receipts
            .entry(receipt.idempotency_key.clone())
            .or_insert(receipt)
            .clone())
    }
}

/// Resource operations must themselves honor `action_key`; this closes the
/// crash window between a successful ownership change and receipt persistence.
#[async_trait]
pub trait SettlementResourcePort: Send + Sync {
    fn reparent_context(
        &self,
        resource: &BackgroundResourceDecl,
        parent_owner: &str,
    ) -> ReparentContext;

    async fn settle_foreground(
        &self,
        resource: &BackgroundResourceDecl,
        action_key: &str,
    ) -> Result<(), AgentControlError>;

    async fn terminate(
        &self,
        resource: &BackgroundResourceDecl,
        reason: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError>;

    async fn reparent(
        &self,
        resource: &BackgroundResourceDecl,
        old_owner: &str,
        new_owner: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError>;
}

#[async_trait]
pub trait SettlementLeasePort: Send + Sync {
    /// Owner-checked deletion. `false` means it was already absent and is a
    /// successful idempotent replay, not an ownership bypass.
    async fn release(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError>;
}

#[derive(Debug, Clone)]
pub struct SettlementRequest {
    pub agent_id: String,
    pub attempt_id: String,
    pub generation: String,
    pub old_owner: String,
    pub parent_owner: Option<String>,
    pub terminal: SettlementTerminal,
    pub lease_keys: Vec<String>,
    pub settled_at_ms: i64,
}

impl SettlementRequest {
    fn validate(&self) -> Result<(), AgentControlError> {
        for (value, label) in [
            (&self.agent_id, "agent ID"),
            (&self.attempt_id, "attempt ID"),
            (&self.generation, "daemon generation"),
            (&self.old_owner, "old owner"),
        ] {
            if value.trim().is_empty() {
                return Err(invalid(format!("settlement {label} must not be empty")));
            }
        }
        if self
            .parent_owner
            .as_ref()
            .is_some_and(|owner| owner.trim().is_empty())
        {
            return Err(invalid("settlement parent owner must not be empty"));
        }
        Ok(())
    }
}

pub struct SettlementEngine {
    receipts: Arc<dyn SettlementReceiptStore>,
    resources: Arc<dyn SettlementResourcePort>,
    leases: Arc<dyn SettlementLeasePort>,
    evidence: Arc<dyn SettlementEvidenceSink>,
    /// Serializes a logical settlement in this daemon generation. External
    /// ports still receive stable action keys for crash-safe replay.
    key_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SettlementEngine {
    pub fn new(
        receipts: Arc<dyn SettlementReceiptStore>,
        resources: Arc<dyn SettlementResourcePort>,
        leases: Arc<dyn SettlementLeasePort>,
        evidence: Arc<dyn SettlementEvidenceSink>,
    ) -> Self {
        Self {
            receipts,
            resources,
            leases,
            evidence,
            key_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Enter Quiescing and return the resource set fixed at spawn/live-run
    /// registration. No new calls are admitted after this point.
    pub async fn quiesce(
        &self,
        live: &LiveAgentRun,
    ) -> Result<Vec<BackgroundResourceDecl>, AgentControlError> {
        self.evidence
            .record(SettlementEvidence::Phase(SettlementPhase::Quiescing))
            .await?;
        Ok(live.begin_quiescing().await)
    }

    pub async fn settle(
        &self,
        request: SettlementRequest,
        resources: Vec<BackgroundResourceDecl>,
    ) -> Result<SettlementReceipt, AgentControlError> {
        request.validate()?;
        let key =
            settlement_idempotency_key(&request.agent_id, &request.attempt_id, &request.generation);
        let key_lock = {
            let mut locks = self.key_locks.lock().await;
            locks
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = key_lock.lock().await;

        if let Some(receipt) = self.receipts.get(&key).await? {
            self.evidence
                .record(SettlementEvidence::IdempotentReplay {
                    idempotency_key: key,
                })
                .await?;
            return Ok(receipt);
        }

        self.evidence
            .record(SettlementEvidence::Phase(SettlementPhase::Settling))
            .await?;

        let mut reparented = Vec::new();
        let mut failures = Vec::new();
        let mut ordered_resources = resources;
        ordered_resources.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        for resource in &ordered_resources {
            let action_key = format!("{key}:resource:{}", resource.resource_id);
            match resource.class {
                AgentResourceClass::ForegroundCommand => {
                    if let Err(error) = self
                        .resources
                        .settle_foreground(resource, &action_key)
                        .await
                    {
                        failures.push(format!(
                            "foreground {} did not settle: {}",
                            resource.resource_id, error.message
                        ));
                        let reason = "foreground settlement failed; forced termination";
                        if let Err(terminate_error) = self
                            .resources
                            .terminate(resource, reason, &action_key)
                            .await
                        {
                            failures.push(format!(
                                "foreground {} termination failed: {}",
                                resource.resource_id, terminate_error.message
                            ));
                        }
                        self.evidence
                            .record(SettlementEvidence::ResourceTerminated {
                                resource_id: resource.resource_id.clone(),
                                class: resource.class,
                                reason: reason.into(),
                            })
                            .await?;
                    }
                }
                AgentResourceClass::BackgroundCommand | AgentResourceClass::NotificationRoute => {
                    let reparent = request.parent_owner.as_deref().and_then(|parent| {
                        let context = self.resources.reparent_context(resource, parent);
                        can_reparent(resource, &context)
                            .map(|()| parent)
                            .map_err(|reason| reason)
                            .ok()
                    });
                    if let Some(parent) = reparent {
                        match self
                            .resources
                            .reparent(resource, &request.old_owner, parent, &action_key)
                            .await
                        {
                            Ok(()) => {
                                let receipt = ReparentReceipt {
                                    resource_id: resource.resource_id.clone(),
                                    class: resource.class,
                                    old_owner: request.old_owner.clone(),
                                    new_owner: parent.to_string(),
                                    reason: "declared survivor accepted by parent authority".into(),
                                    at_ms: request.settled_at_ms,
                                };
                                self.evidence
                                    .record(SettlementEvidence::Reparented(receipt.clone()))
                                    .await?;
                                reparented.push(receipt);
                                continue;
                            }
                            Err(error) => failures.push(format!(
                                "reparent {} failed: {}",
                                resource.resource_id, error.message
                            )),
                        }
                    }

                    let reason = reparent_denial_reason(
                        self.resources.as_ref(),
                        resource,
                        request.parent_owner.as_deref(),
                    );
                    if let Err(error) = self
                        .resources
                        .terminate(resource, &reason, &action_key)
                        .await
                    {
                        failures.push(format!(
                            "resource {} termination failed: {}",
                            resource.resource_id, error.message
                        ));
                    }
                    self.evidence
                        .record(SettlementEvidence::ResourceTerminated {
                            resource_id: resource.resource_id.clone(),
                            class: resource.class,
                            reason,
                        })
                        .await?;
                }
                AgentResourceClass::Worktree => {
                    let reason = "child-owned worktree requires cleanup or recovery";
                    if let Err(error) = self
                        .resources
                        .terminate(resource, reason, &action_key)
                        .await
                    {
                        failures.push(format!(
                            "worktree {} cleanup failed: {}",
                            resource.resource_id, error.message
                        ));
                    }
                    self.evidence
                        .record(SettlementEvidence::ResourceTerminated {
                            resource_id: resource.resource_id.clone(),
                            class: resource.class,
                            reason: reason.into(),
                        })
                        .await?;
                }
            }
        }

        let mut released_leases = Vec::new();
        let mut lease_keys = request.lease_keys;
        lease_keys.sort();
        lease_keys.dedup();
        for lease_key in lease_keys {
            match self.leases.release(&lease_key, &request.old_owner).await {
                Ok(true) => released_leases.push(lease_key),
                Ok(false) => {}
                Err(error) => failures.push(format!(
                    "lease {lease_key} release failed: {}",
                    error.message
                )),
            }
        }

        let terminal = if failures.is_empty() {
            request.terminal
        } else {
            SettlementTerminal::Failed {
                reason: failures.join("; "),
            }
        };
        let receipt = SettlementReceipt {
            agent_id: request.agent_id,
            attempt_id: request.attempt_id,
            generation: request.generation,
            terminal,
            released_leases,
            reparented,
            settled_at_ms: request.settled_at_ms,
            idempotency_key: key,
        };
        let receipt = self.receipts.put_if_absent(receipt).await?;
        self.evidence
            .record(SettlementEvidence::Phase(SettlementPhase::Terminal))
            .await?;
        Ok(receipt)
    }
}

fn reparent_denial_reason(
    resources: &dyn SettlementResourcePort,
    resource: &BackgroundResourceDecl,
    parent_owner: Option<&str>,
) -> String {
    let Some(parent) = parent_owner else {
        return "no parent authority is available".into();
    };
    can_reparent(resource, &resources.reparent_context(resource, parent))
        .err()
        .unwrap_or_else(|| "reparent operation failed".into())
}

/// Recovery policy for resources whose settlement was interrupted by a crash.
pub fn recovery_disposition(decision: AgentRecoveryDecision) -> RecoveryResourceDisposition {
    match decision {
        AgentRecoveryDecision::Resume => RecoveryResourceDisposition::RetainForResume,
        AgentRecoveryDecision::Finalize => RecoveryResourceDisposition::ReplaySettlement,
        AgentRecoveryDecision::Interrupt | AgentRecoveryDecision::Reclaim => {
            RecoveryResourceDisposition::TerminateAndReclaim
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResourceDisposition {
    RetainForResume,
    ReplaySettlement,
    TerminateAndReclaim,
}

fn invalid(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.into(),
    }
}

fn persistence(error: impl std::fmt::Display) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Persistence,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex as ParkingMutex;

    #[test]
    fn parent_budget_acceptance_is_closed_until_transfer_is_published() {
        let gate = ParentBudgetAcceptance::default();
        assert!(!gate.read());
        gate.publish(true);
        assert!(gate.read());
    }

    #[derive(Default)]
    struct FakeAdmission {
        settled: usize,
        revoked: usize,
    }

    #[async_trait]
    impl AgentAdmissionLease for FakeAdmission {
        async fn mark_running(&mut self) -> Result<(), AgentControlError> {
            Ok(())
        }

        async fn settle(&mut self, _usage: &fabric::AttemptUsage) -> Result<(), AgentControlError> {
            self.settled += 1;
            Ok(())
        }

        async fn revoke(&mut self) -> Result<(), AgentControlError> {
            self.revoked += 1;
            Ok(())
        }

        async fn transfer_remaining_to(
            &mut self,
            _parent: fabric::AgentId,
            _usage: &fabric::AttemptUsage,
        ) -> Result<fabric::BudgetTransferReceipt, AgentControlError> {
            Err(invalid("fake admission transfer is unavailable"))
        }
    }

    #[derive(Default)]
    struct FakeResources {
        context: ParkingMutex<Option<ReparentContext>>,
        foreground: ParkingMutex<Vec<String>>,
        terminated: ParkingMutex<Vec<String>>,
        reparented: ParkingMutex<Vec<(String, String, String)>>,
        fail_foreground: ParkingMutex<bool>,
    }

    impl FakeResources {
        fn allowing() -> Self {
            Self {
                context: ParkingMutex::new(Some(ReparentContext {
                    parent_authority_covers: true,
                    parent_budget_accepts: true,
                    notification_route_transferable: true,
                })),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl SettlementResourcePort for FakeResources {
        fn reparent_context(
            &self,
            _resource: &BackgroundResourceDecl,
            _parent_owner: &str,
        ) -> ReparentContext {
            self.context.lock().clone().unwrap_or(ReparentContext {
                parent_authority_covers: false,
                parent_budget_accepts: false,
                notification_route_transferable: false,
            })
        }

        async fn settle_foreground(
            &self,
            resource: &BackgroundResourceDecl,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.foreground.lock().push(resource.resource_id.clone());
            if *self.fail_foreground.lock() {
                Err(runtime("foreground still live"))
            } else {
                Ok(())
            }
        }

        async fn terminate(
            &self,
            resource: &BackgroundResourceDecl,
            _reason: &str,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.terminated.lock().push(resource.resource_id.clone());
            Ok(())
        }

        async fn reparent(
            &self,
            resource: &BackgroundResourceDecl,
            old_owner: &str,
            new_owner: &str,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.reparented.lock().push((
                resource.resource_id.clone(),
                old_owner.into(),
                new_owner.into(),
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeLeases {
        calls: ParkingMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl SettlementLeasePort for FakeLeases {
        async fn release(
            &self,
            lease_key: &str,
            expected_owner: &str,
        ) -> Result<bool, AgentControlError> {
            let mut calls = self.calls.lock();
            let call = (lease_key.to_string(), expected_owner.to_string());
            if calls.contains(&call) {
                return Ok(false);
            }
            calls.push(call);
            Ok(true)
        }
    }

    #[derive(Default)]
    struct FakeEvidence(ParkingMutex<Vec<SettlementEvidence>>);

    #[async_trait]
    impl SettlementEvidenceSink for FakeEvidence {
        async fn record(&self, evidence: SettlementEvidence) -> Result<(), AgentControlError> {
            self.0.lock().push(evidence);
            Ok(())
        }
    }

    struct Harness {
        engine: SettlementEngine,
        resources: Arc<FakeResources>,
        leases: Arc<FakeLeases>,
        evidence: Arc<FakeEvidence>,
    }

    fn harness(resources: FakeResources) -> Harness {
        let resources = Arc::new(resources);
        let leases = Arc::new(FakeLeases::default());
        let evidence = Arc::new(FakeEvidence::default());
        Harness {
            engine: SettlementEngine::new(
                Arc::new(InMemorySettlementReceiptStore::default()),
                resources.clone(),
                leases.clone(),
                evidence.clone(),
            ),
            resources,
            leases,
            evidence,
        }
    }

    fn request() -> SettlementRequest {
        SettlementRequest {
            agent_id: "agent".into(),
            attempt_id: "attempt".into(),
            generation: "generation".into(),
            old_owner: "child".into(),
            parent_owner: Some("parent".into()),
            terminal: SettlementTerminal::Completed,
            lease_keys: vec!["execution".into(), "admission".into()],
            settled_at_ms: 42,
        }
    }

    fn resource(
        id: &str,
        class: AgentResourceClass,
        survive_child: bool,
    ) -> BackgroundResourceDecl {
        BackgroundResourceDecl {
            resource_id: id.into(),
            class,
            survive_child,
        }
    }

    #[tokio::test]
    async fn settlement_replay_does_not_repeat_release_or_reparent() {
        let harness = harness(FakeResources::allowing());
        let resources = vec![resource(
            "background",
            AgentResourceClass::BackgroundCommand,
            true,
        )];
        let first = harness
            .engine
            .settle(request(), resources.clone())
            .await
            .unwrap();
        let replay = harness.engine.settle(request(), resources).await.unwrap();

        assert_eq!(first, replay);
        assert_eq!(harness.resources.reparented.lock().len(), 1);
        assert_eq!(harness.leases.calls.lock().len(), 2);
        assert!(harness
            .evidence
            .0
            .lock()
            .iter()
            .any(|event| matches!(event, SettlementEvidence::IdempotentReplay { .. })));
    }

    #[tokio::test]
    async fn undeclared_survivor_is_killed_and_never_reparented() {
        let harness = harness(FakeResources::allowing());
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "background",
                    AgentResourceClass::BackgroundCommand,
                    false,
                )],
            )
            .await
            .unwrap();
        assert!(receipt.reparented.is_empty());
        assert_eq!(&*harness.resources.terminated.lock(), &["background"]);
    }

    #[tokio::test]
    async fn budget_or_authority_denial_kills_resource_with_evidence() {
        let resources = FakeResources::allowing();
        resources
            .context
            .lock()
            .as_mut()
            .unwrap()
            .parent_budget_accepts = false;
        let harness = harness(resources);
        harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "background",
                    AgentResourceClass::BackgroundCommand,
                    true,
                )],
            )
            .await
            .unwrap();
        assert_eq!(&*harness.resources.terminated.lock(), &["background"]);
        assert!(harness.evidence.0.lock().iter().any(|event| matches!(
            event,
            SettlementEvidence::ResourceTerminated { reason, .. }
                if reason.contains("budget")
        )));
    }

    #[tokio::test]
    async fn transferable_notification_route_is_reparented_with_receipt() {
        let harness = harness(FakeResources::allowing());
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "notify",
                    AgentResourceClass::NotificationRoute,
                    true,
                )],
            )
            .await
            .unwrap();
        assert_eq!(receipt.reparented.len(), 1);
        assert_eq!(receipt.reparented[0].old_owner, "child");
        assert_eq!(receipt.reparented[0].new_owner, "parent");
    }

    #[tokio::test]
    async fn foreground_failure_forces_termination_before_terminal() {
        let resources = FakeResources::allowing();
        *resources.fail_foreground.lock() = true;
        let harness = harness(resources);
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "foreground",
                    AgentResourceClass::ForegroundCommand,
                    false,
                )],
            )
            .await
            .unwrap();
        assert!(matches!(
            receipt.terminal,
            SettlementTerminal::Failed { .. }
        ));
        assert_eq!(&*harness.resources.terminated.lock(), &["foreground"]);
    }

    #[test]
    fn recovery_decisions_distinguish_resume_finalize_and_reclaim() {
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Resume),
            RecoveryResourceDisposition::RetainForResume
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Finalize),
            RecoveryResourceDisposition::ReplaySettlement
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Reclaim),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Interrupt),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
    }

    #[tokio::test]
    async fn admission_adapter_settles_success_and_revokes_failure() {
        let usage = fabric::AttemptUsage::default();
        let mut succeeded = FakeAdmission::default();
        settle_admission(&mut succeeded, &SettlementTerminal::Completed, Some(&usage))
            .await
            .unwrap();
        assert_eq!((succeeded.settled, succeeded.revoked), (1, 0));

        let mut failed = FakeAdmission::default();
        settle_admission(
            &mut failed,
            &SettlementTerminal::Failed {
                reason: "runtime".into(),
            },
            Some(&usage),
        )
        .await
        .unwrap();
        assert_eq!((failed.settled, failed.revoked), (0, 1));
    }

    #[test]
    fn memory_flush_error_prevents_completed_receipt() {
        let terminal = terminal_with_memory_flush(
            SettlementTerminal::Completed,
            Some(runtime("vault unavailable")),
        );
        assert!(matches!(
            terminal,
            SettlementTerminal::Failed { reason } if reason.contains("vault unavailable")
        ));
    }

    fn runtime(message: &str) -> AgentControlError {
        AgentControlError {
            kind: AgentControlErrorKind::Runtime,
            message: message.into(),
        }
    }
}
