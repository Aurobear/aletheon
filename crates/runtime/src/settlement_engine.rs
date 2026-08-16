//! Runtime-owned Agent settlement state machine (RA-05).
//!
//! The engine owns ordering, generation fencing, idempotency and terminal
//! settlement policy. Concrete resources, leases and evidence are injected
//! through narrow Runtime ports; host adapters provide their implementations.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ::contracts::{
    can_reparent, settlement_idempotency_key, AgentControlError, AgentControlErrorKind,
    AgentResourceClass, BackgroundResourceDecl, ReparentReceipt, SettlementPhase,
    SettlementReceipt, SettlementTerminal,
};
use tokio::sync::Mutex;

use crate::generation_fence::GenerationFence;
use crate::{
    SettlementEvidence, SettlementEvidenceSink, SettlementLeasePort, SettlementQuiescePort,
    SettlementReceiptStore, SettlementResourcePort,
};

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
    key_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    metrics: Arc<SettlementMetrics>,
    generation_fence: GenerationFence,
}

#[derive(Debug, Default)]
pub struct SettlementMetrics {
    duration_ms: AtomicU64,
    reparent_background_total: AtomicU64,
    reparent_notification_total: AtomicU64,
    reparent_denied_total: AtomicU64,
    idempotent_replay_total: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettlementMetricSnapshot {
    pub settlement_duration_ms: u64,
    pub reparent_background_command_total: u64,
    pub reparent_notification_route_total: u64,
    pub reparent_denied_total: u64,
    pub settlement_idempotent_replay_total: u64,
}

impl SettlementMetricSnapshot {
    pub fn named(self) -> [(String, String); 5] {
        [
            (
                "settlement_duration_ms".into(),
                self.settlement_duration_ms.to_string(),
            ),
            (
                "reparent_total.background_command".into(),
                self.reparent_background_command_total.to_string(),
            ),
            (
                "reparent_total.notification_route".into(),
                self.reparent_notification_route_total.to_string(),
            ),
            (
                "reparent_denied_total".into(),
                self.reparent_denied_total.to_string(),
            ),
            (
                "settlement_idempotent_replay_total".into(),
                self.settlement_idempotent_replay_total.to_string(),
            ),
        ]
    }
}

impl SettlementMetrics {
    pub fn snapshot(&self) -> SettlementMetricSnapshot {
        SettlementMetricSnapshot {
            settlement_duration_ms: self.duration_ms.load(Ordering::Relaxed),
            reparent_background_command_total: self
                .reparent_background_total
                .load(Ordering::Relaxed),
            reparent_notification_route_total: self
                .reparent_notification_total
                .load(Ordering::Relaxed),
            reparent_denied_total: self.reparent_denied_total.load(Ordering::Relaxed),
            settlement_idempotent_replay_total: self
                .idempotent_replay_total
                .load(Ordering::Relaxed),
        }
    }
}

impl SettlementEngine {
    pub fn new(
        receipts: Arc<dyn SettlementReceiptStore>,
        resources: Arc<dyn SettlementResourcePort>,
        leases: Arc<dyn SettlementLeasePort>,
        evidence: Arc<dyn SettlementEvidenceSink>,
    ) -> Self {
        Self::with_metrics(
            receipts,
            resources,
            leases,
            evidence,
            Arc::new(SettlementMetrics::default()),
        )
    }

    pub fn with_metrics(
        receipts: Arc<dyn SettlementReceiptStore>,
        resources: Arc<dyn SettlementResourcePort>,
        leases: Arc<dyn SettlementLeasePort>,
        evidence: Arc<dyn SettlementEvidenceSink>,
        metrics: Arc<SettlementMetrics>,
    ) -> Self {
        Self {
            receipts,
            resources,
            leases,
            evidence,
            key_locks: Mutex::new(HashMap::new()),
            metrics,
            generation_fence: GenerationFence::default(),
        }
    }

    pub fn with_generation(mut self, generation: impl Into<String>) -> Self {
        self.generation_fence = GenerationFence::bind(generation);
        self
    }

    pub async fn quiesce(
        &self,
        live: &dyn SettlementQuiescePort,
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
        let started = Instant::now();
        request.validate()?;
        if let Some((expected, received)) = self.generation_fence.rejection(&request.generation) {
            self.evidence
                .record(SettlementEvidence::GenerationRejected(
                    expected.into(),
                    received.into(),
                ))
                .await?;
            return Err(invalid(format!(
                "stale daemon generation: expected {expected}, received {received}"
            )));
        }
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
            self.metrics
                .idempotent_replay_total
                .fetch_add(1, Ordering::Relaxed);
            self.evidence
                .record(SettlementEvidence::IdempotentReplay {
                    idempotency_key: key,
                })
                .await?;
            self.metrics.duration_ms.store(
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
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
                        can_reparent(resource, &context).ok().map(|()| parent)
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
                                match resource.class {
                                    AgentResourceClass::BackgroundCommand => {
                                        self.metrics
                                            .reparent_background_total
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                    AgentResourceClass::NotificationRoute => {
                                        self.metrics
                                            .reparent_notification_total
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                    _ => {}
                                }
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
                    self.metrics
                        .reparent_denied_total
                        .fetch_add(1, Ordering::Relaxed);
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
        self.metrics.duration_ms.store(
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
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

fn invalid(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.into(),
    }
}
