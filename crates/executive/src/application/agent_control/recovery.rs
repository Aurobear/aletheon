//! Durable startup reconciliation for open Agent runs.

use std::sync::Arc;

use fabric::{
    AgentControlError, AgentControlErrorKind, AgentRecoveryDecision, AgentRecoveryReceipt,
    AgentRunStatus, RuntimeResumability,
};
use sha2::{Digest, Sha256};

use super::{AgentRunRecord, AgentRunRepository};

pub const MAX_STARTUP_RECOVERY_ROWS: usize = 1_000;

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
}

impl AgentRecoveryReport {
    pub fn ready(&self) -> bool {
        self.recovery_failed == 0 && self.unreconciled == 0
    }
}

pub struct AgentRecoveryCoordinator {
    repository: Arc<dyn AgentRunRepository>,
    daemon_generation: String,
    recovered_at_ms: i64,
}

impl AgentRecoveryCoordinator {
    pub fn new(
        repository: Arc<dyn AgentRunRepository>,
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
        let decision = run.recovery.as_ref().map_or_else(
            || Self::decide(run, observation),
            |receipt| receipt.decision,
        );
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
        if run.recovery.is_none() {
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
                let mut terminal = observation.operation_terminal.ok_or_else(|| {
                    AgentControlError::invalid("finalize recovery lacks terminal Kernel state")
                })?;
                if terminal == AgentRunStatus::Succeeded && run.snapshot.result.is_none() {
                    terminal = AgentRunStatus::Failed;
                }
                self.repository
                    .transition(
                        run.agent_id(),
                        run.status(),
                        terminal,
                        run.snapshot.result.clone(),
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
}
