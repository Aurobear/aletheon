//! Durable startup reconciliation for open Agent runs.

use std::sync::Arc;

use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentHandle, AgentRecoveryDecision,
    AgentRecoveryReceipt, AgentRunStatus, AgentSpawnRequest, RuntimeResumability,
};
use sha2::{Digest, Sha256};

use crate::agent_repository::{AgentRunProjection, AgentRunRecord};

pub const MAX_STARTUP_RECOVERY_ROWS: usize = 1_000;

/// Opaque checkpoint resume input passed to the already-pinned host launcher.
/// Runtime owns the identity-bearing handle and request contract; the
/// checkpoint reference remains adapter-owned data.
#[derive(Debug, Clone)]
pub struct AgentRecoveryRuntimeInput {
    pub handle: AgentHandle,
    pub request: AgentSpawnRequest,
    pub checkpoint_reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeProcessReclaimOutcome {
    Reclaimed,
    AlreadyExited,
    IdentityReused,
}

#[async_trait::async_trait]
pub trait RuntimeProcessSupervisor: Send + Sync {
    async fn reclaim(
        &self,
        identity: ::contracts::RuntimeProcessId,
    ) -> Result<RuntimeProcessReclaimOutcome, AgentControlError>;
}

#[derive(Debug, Default)]
pub struct FailClosedRuntimeProcessSupervisor;

#[async_trait::async_trait]
impl RuntimeProcessSupervisor for FailClosedRuntimeProcessSupervisor {
    async fn reclaim(
        &self,
        _identity: ::contracts::RuntimeProcessId,
    ) -> Result<RuntimeProcessReclaimOutcome, AgentControlError> {
        Err(AgentControlError::invalid(
            "durable runtime process exists without a configured supervisor",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRecoveryObservation {
    pub process_live: bool,
    pub operation_terminal: Option<AgentRunStatus>,
    pub checkpoint_available: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRecoveryReport {
    pub open_rows: usize,
    pub interrupted: usize,
    pub resumed: usize,
    pub finalized: usize,
    pub recovery_failed: usize,
    pub unreconciled: usize,
    pub orphan_reclaimed: usize,
    pub orphan_already_exited: usize,
    pub orphan_identity_reused: usize,
}

/// Host-owned observations and side effects for startup reconciliation.
///
/// Runtime owns the durable decision, transition, pagination, and report
/// semantics.  A host adapter supplies only environment-specific observation
/// (Kernel/process state) and the concrete resume/settlement operation.  This
/// keeps the Executive compatibility facade from becoming a second recovery
/// state machine.
#[async_trait::async_trait]
pub trait AgentRecoveryHost: Send + Sync {
    async fn observe(
        &self,
        run: &AgentRunRecord,
    ) -> Result<AgentRecoveryObservation, AgentControlError>;

    /// Apply the host side of a Runtime decision.  `true` is returned only
    /// when a checkpoint resume was actually accepted; interrupt/finalize
    /// return `false` because they are already durable transitions.
    async fn apply(
        &self,
        run: &AgentRunRecord,
        decision: AgentRecoveryDecision,
        observation: AgentRecoveryObservation,
    ) -> Result<bool, AgentControlError>;

    /// Optional counts for host-side durable process reclamation performed
    /// while observing a run.  Runtime folds these into the canonical report.
    fn process_reclaim_counts(&self) -> (usize, usize, usize) {
        (0, 0, 0)
    }
}

impl AgentRecoveryReport {
    pub fn ready(&self) -> bool {
        self.recovery_failed == 0 && self.unreconciled == 0
    }
}

pub struct AgentRecoveryCoordinator {
    repository: Arc<dyn AgentRunProjection>,
    daemon_generation: String,
    recovered_at_ms: i64,
}

impl AgentRecoveryCoordinator {
    pub fn new(
        repository: Arc<dyn AgentRunProjection>,
        daemon_generation: impl Into<String>,
        recovered_at_ms: i64,
    ) -> Result<Self, AgentControlError> {
        let daemon_generation = daemon_generation.into();
        if daemon_generation.trim().is_empty() {
            return Err(AgentControlError::invalid("daemon generation is required"));
        }
        Ok(Self {
            repository,
            daemon_generation,
            recovered_at_ms,
        })
    }

    pub fn decide(
        run: &AgentRunRecord,
        observation: AgentRecoveryObservation,
    ) -> AgentRecoveryDecision {
        if observation.operation_terminal.is_some() {
            return AgentRecoveryDecision::Finalize;
        }
        if observation.process_live
            && observation.checkpoint_available
            && matches!(run.resumability, RuntimeResumability::Checkpointed { .. })
        {
            AgentRecoveryDecision::Resume
        } else {
            AgentRecoveryDecision::Interrupt
        }
    }

    pub async fn recover_one(
        &self,
        run: &AgentRunRecord,
        observation: AgentRecoveryObservation,
    ) -> Result<AgentRecoveryDecision, AgentControlError> {
        let terminal_receipt = self.repository.terminal_receipt(run.agent_id()).await?;
        let decision = if terminal_receipt.is_some() {
            AgentRecoveryDecision::Finalize
        } else {
            run.recovery
                .as_ref()
                .filter(|receipt| receipt.daemon_generation == self.daemon_generation)
                .map_or_else(
                    || Self::decide(run, observation),
                    |receipt| receipt.decision,
                )
        };
        let idempotency_key = format!(
            "sha256:{:x}",
            Sha256::digest(
                format!(
                    "{}:{}:{}:{decision:?}",
                    run.agent_id().0,
                    run.snapshot.handle.process_id.0,
                    run.version
                )
                .as_bytes()
            )
        );
        if run
            .recovery
            .as_ref()
            .is_none_or(|receipt| receipt.daemon_generation != self.daemon_generation)
        {
            let receipt = AgentRecoveryReceipt {
                decision,
                daemon_generation: self.daemon_generation.clone(),
                recovered_at_ms: self.recovered_at_ms,
                idempotency_key,
            };
            // The decision is durable before any lifecycle action.
            self.repository
                .record_recovery(run.agent_id(), &receipt)
                .await?;
        }
        match decision {
            AgentRecoveryDecision::Interrupt => {
                self.repository
                    .transition(
                        run.agent_id(),
                        run.status(),
                        AgentRunStatus::Interrupted,
                        None,
                        Some("daemon restart interrupted non-resumable Agent work".into()),
                        self.recovered_at_ms,
                    )
                    .await?;
            }
            AgentRecoveryDecision::Finalize => {
                let mut terminal = terminal_receipt
                    .as_ref()
                    .map(|receipt| receipt.status)
                    .or(observation.operation_terminal)
                    .ok_or_else(|| {
                        AgentControlError::invalid("finalize recovery lacks terminal evidence")
                    })?;
                let result = terminal_receipt
                    .as_ref()
                    .and_then(|receipt| receipt.result.clone())
                    .or_else(|| run.snapshot.result.clone());
                if terminal == AgentRunStatus::Succeeded && result.is_none() {
                    terminal = AgentRunStatus::Failed;
                }
                self.repository
                    .transition(
                        run.agent_id(),
                        run.status(),
                        terminal,
                        result,
                        (terminal == AgentRunStatus::Failed)
                            .then(|| "Kernel completed without a persisted Agent result".into()),
                        self.recovered_at_ms,
                    )
                    .await?;
            }
            AgentRecoveryDecision::Resume => {
                // The durable checkpoint remains owned by the same Agent ID.
                // A runtime-specific supervisor consumes it; launch() is never
                // called here because that would replay ambiguous provider work.
            }
            AgentRecoveryDecision::Reclaim => {
                return Err(AgentControlError {
                    kind: AgentControlErrorKind::InvalidRequest,
                    message: "resource reclaim is not a run reconciliation decision".into(),
                });
            }
        }
        Ok(decision)
    }

    pub async fn recover_with<F>(
        &self,
        mut observe: F,
    ) -> Result<AgentRecoveryReport, AgentControlError>
    where
        F: FnMut(&AgentRunRecord) -> AgentRecoveryObservation,
    {
        let runs = self.repository.list_open(MAX_STARTUP_RECOVERY_ROWS).await?;
        let mut report = AgentRecoveryReport {
            open_rows: runs.len(),
            ..Default::default()
        };
        for run in runs {
            match self.recover_one(&run, observe(&run)).await {
                Ok(AgentRecoveryDecision::Interrupt) => report.interrupted += 1,
                Ok(AgentRecoveryDecision::Resume) => report.resumed += 1,
                Ok(AgentRecoveryDecision::Finalize) => report.finalized += 1,
                Ok(AgentRecoveryDecision::Reclaim) => report.recovery_failed += 1,
                Err(_) => report.recovery_failed += 1,
            }
        }
        report.unreconciled = self
            .repository
            .list_open(MAX_STARTUP_RECOVERY_ROWS)
            .await?
            .len();
        // Resumed checkpoint rows intentionally remain open but are reconciled.
        report.unreconciled = report.unreconciled.saturating_sub(report.resumed);
        Ok(report)
    }

    /// Reconcile open runs through the Runtime-owned coordinator while a host
    /// adapter contributes only process observations and concrete actions.
    /// Pagination, durable decision ordering, and the unreconciled fence stay
    /// in Runtime so a compatibility facade cannot silently diverge.
    pub async fn reconcile_with<H>(
        &self,
        host: &H,
    ) -> Result<AgentRecoveryReport, AgentControlError>
    where
        H: AgentRecoveryHost,
    {
        let mut report = AgentRecoveryReport::default();
        let mut cursor = None;
        loop {
            let runs = self
                .repository
                .list_open_after(cursor, MAX_STARTUP_RECOVERY_ROWS)
                .await?;
            if runs.is_empty() {
                break;
            }
            report.open_rows = report.open_rows.saturating_add(runs.len());
            cursor = runs
                .last()
                .map(|run| (run.snapshot.created_at_ms, run.agent_id()));
            for run in runs {
                let observation = host.observe(&run).await?;
                match self.recover_one(&run, observation).await {
                    Ok(AgentRecoveryDecision::Interrupt) => {
                        host.apply(&run, AgentRecoveryDecision::Interrupt, observation)
                            .await?;
                        report.interrupted = report.interrupted.saturating_add(1);
                    }
                    Ok(AgentRecoveryDecision::Resume) => {
                        if host
                            .apply(&run, AgentRecoveryDecision::Resume, observation)
                            .await?
                        {
                            report.resumed = report.resumed.saturating_add(1);
                        } else {
                            report.recovery_failed = report.recovery_failed.saturating_add(1);
                        }
                    }
                    Ok(AgentRecoveryDecision::Finalize) => {
                        host.apply(&run, AgentRecoveryDecision::Finalize, observation)
                            .await?;
                        report.finalized = report.finalized.saturating_add(1);
                    }
                    Ok(AgentRecoveryDecision::Reclaim) => {
                        report.recovery_failed = report.recovery_failed.saturating_add(1);
                    }
                    Err(error) => {
                        tracing::warn!(
                            agent_id = %run.agent_id().0,
                            %error,
                            "Runtime startup recovery decision failed"
                        );
                        report.recovery_failed = report.recovery_failed.saturating_add(1);
                    }
                }
            }
        }

        self.refresh_unreconciled(&mut report).await?;
        let (reclaimed, already_exited, identity_reused) = host.process_reclaim_counts();
        report.orphan_reclaimed = report.orphan_reclaimed.saturating_add(reclaimed);
        report.orphan_already_exited = report.orphan_already_exited.saturating_add(already_exited);
        report.orphan_identity_reused = report
            .orphan_identity_reused
            .saturating_add(identity_reused);
        Ok(report)
    }

    /// Refresh the open-row fence after a host adapter has handled additional
    /// Runtime-only orphan identities.
    pub async fn refresh_unreconciled(
        &self,
        report: &mut AgentRecoveryReport,
    ) -> Result<(), AgentControlError> {
        report.unreconciled = 0;
        let mut cursor = None;
        loop {
            let runs = self
                .repository
                .list_open_after(cursor, MAX_STARTUP_RECOVERY_ROWS)
                .await?;
            if runs.is_empty() {
                break;
            }
            cursor = runs
                .last()
                .map(|run| (run.snapshot.created_at_ms, run.agent_id()));
            report.unreconciled = report.unreconciled.saturating_add(
                runs.into_iter()
                    .filter(|run| {
                        !matches!(
                            run.recovery.as_ref(),
                            Some(receipt)
                                if receipt.daemon_generation == self.daemon_generation
                                    && receipt.decision == AgentRecoveryDecision::Resume
                        )
                    })
                    .count(),
            );
        }
        Ok(())
    }
}
