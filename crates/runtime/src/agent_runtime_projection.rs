//! Runtime authority + host projection bridge for AgentRun lifecycle.
//!
//! The legacy `agent_control.db` schema remains useful for rich request,
//! workspace, lease, payload, and admin reads. It is explicitly a projection,
//! however: lifecycle admission, start/terminal fencing, recovery receipts and
//! mailbox delivery observations are appended to Runtime's AgentStream first.
//! This adapter is the only production composition point that combines those
//! two concerns; callers depend on `AgentRunProjection`, not on SQL authority.

use crate::{
    AgentMailboxDelivery, AgentMessageRecord, AgentResourceLease, AgentRunId, AgentRunProjection,
    AgentRunRecord, AgentTerminalReceipt, Generation, RuntimeAgentSupervisor, RuntimeError,
    SessionId, TurnTerminal,
};
use ::contracts::{
    AgentControlError, AgentId, AgentMessageDeliveryState, AgentMessagePayload,
    AgentRecoveryReceipt, AgentResult, AgentRunStatus, RuntimeProcessId,
};
use async_trait::async_trait;
use std::sync::Arc;

fn runtime_agent(agent: AgentId) -> AgentRunId {
    AgentRunId(agent.0.to_string())
}

fn projected_runtime_session(run: &AgentRunRecord) -> SessionId {
    SessionId(run.root_agent_id().0.to_string())
}

fn terminal_for(
    status: AgentRunStatus,
    result: Option<&AgentResult>,
    error: Option<&str>,
) -> Option<TurnTerminal> {
    match status {
        AgentRunStatus::Succeeded => Some(TurnTerminal::Completed),
        AgentRunStatus::Cancelled | AgentRunStatus::Interrupted => Some(TurnTerminal::Interrupted),
        AgentRunStatus::Failed => Some(TurnTerminal::Failed {
            message: error
                .map(str::to_owned)
                .or_else(|| result.map(|value| value.output.chars().take(1024).collect()))
                .unwrap_or_else(|| "Agent runtime failed".into()),
        }),
        AgentRunStatus::Queued | AgentRunStatus::Running | AgentRunStatus::Waiting => None,
    }
}

fn runtime_delivery(value: AgentMessageDeliveryState) -> AgentMailboxDelivery {
    match value {
        AgentMessageDeliveryState::Pending => AgentMailboxDelivery::Pending,
        AgentMessageDeliveryState::Delivered => AgentMailboxDelivery::Delivered,
        AgentMessageDeliveryState::Rejected => AgentMailboxDelivery::Rejected,
    }
}

/// One-way authority adapter. SQL is intentionally named and typed as a
/// projection; all write methods that describe Agent lifecycle call Runtime
/// before forwarding the compatibility projection write.
pub struct RuntimeAgentRunProjection {
    projection: Arc<dyn AgentRunProjection>,
    supervisor: Arc<RuntimeAgentSupervisor>,
}

impl RuntimeAgentRunProjection {
    pub fn new(
        projection: Arc<dyn AgentRunProjection>,
        supervisor: Arc<RuntimeAgentSupervisor>,
    ) -> Self {
        Self {
            projection,
            supervisor,
        }
    }

    async fn ensure_started(&self, run: &AgentRunRecord) -> Result<(), AgentControlError> {
        let agent = runtime_agent(run.agent_id());
        let session = self
            .supervisor
            .session(&agent)
            .unwrap_or_else(|_| projected_runtime_session(run));
        if self.supervisor.terminal(&agent).is_some() {
            return Ok(());
        }
        let generation = self.supervisor.generation(&agent).unwrap_or(Generation(1));
        if self.supervisor.generation(&agent).is_err() {
            self.supervisor
                .record_accepted(
                    session.clone(),
                    agent.clone(),
                    generation.clone(),
                    Some(run.snapshot.handle.runtime_id.0.clone()),
                )
                .await
                .map_err(runtime_error)?;
        }
        self.supervisor
            .record_started(
                session,
                agent,
                generation,
                Some(run.snapshot.handle.runtime_id.0.clone()),
            )
            .await
            .map_err(runtime_error)
    }
}

fn runtime_error(error: RuntimeError) -> AgentControlError {
    AgentControlError::invalid(format!(
        "Runtime Agent authority rejected projection write: {error}"
    ))
}

#[async_trait]
impl AgentRunProjection for RuntimeAgentRunProjection {
    async fn create(&self, run: &AgentRunRecord) -> Result<(), AgentControlError> {
        let agent = runtime_agent(run.agent_id());
        let session = self
            .supervisor
            .session(&agent)
            .unwrap_or_else(|_| projected_runtime_session(run));
        let generation = self.supervisor.generation(&agent).unwrap_or(Generation(1));
        self.supervisor
            .record_accepted(
                session,
                agent.clone(),
                generation,
                Some(run.snapshot.handle.runtime_id.0.clone()),
            )
            .await
            .map_err(runtime_error)?;
        if let Err(error) = self.projection.create(run).await {
            let _ = self
                .supervisor
                .record_settled(
                    &agent,
                    TurnTerminal::Failed {
                        message: "Agent SQL projection admission failed".into(),
                    },
                )
                .await;
            return Err(error);
        }
        Ok(())
    }

    async fn transition(
        &self,
        agent: AgentId,
        expected: AgentRunStatus,
        next: AgentRunStatus,
        result: Option<AgentResult>,
        error: Option<String>,
        now_ms: i64,
    ) -> Result<AgentRunRecord, AgentControlError> {
        let current = self
            .projection
            .get(agent)
            .await?
            .ok_or_else(|| AgentControlError::invalid("Agent projection row was not found"))?;
        if current.status() != expected {
            return Err(AgentControlError::invalid(format!(
                "Agent projection transition expected {expected:?}, found {:?}",
                current.status()
            )));
        }
        let agent_run = runtime_agent(agent);
        if next == AgentRunStatus::Running {
            self.ensure_started(&current).await?;
        } else if let Some(terminal) = terminal_for(next, result.as_ref(), error.as_deref()) {
            // A recovered Runtime terminal is already authoritative. The SQL
            // row may be catching up after a prior interrupted bootstrap, so
            // do not attempt a second settlement (which the Runtime reducer
            // correctly rejects as a late terminal write).
            if self.supervisor.terminal(&agent_run).is_none() {
                self.ensure_started(&current).await?;
                self.supervisor
                    .record_settled(&agent_run, terminal)
                    .await
                    .map_err(runtime_error)?;
            }
        }
        self.projection
            .transition(agent, expected, next, result, error, now_ms)
            .await
    }

    async fn get(&self, agent: AgentId) -> Result<Option<AgentRunRecord>, AgentControlError> {
        self.projection.get(agent).await
    }

    async fn list_root(
        &self,
        root: AgentId,
        status: Option<AgentRunStatus>,
        limit: usize,
    ) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        self.projection.list_root(root, status, limit).await
    }

    async fn list_open(&self, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        self.projection.list_open(limit).await
    }

    async fn list_open_after(
        &self,
        after: Option<(i64, AgentId)>,
        limit: usize,
    ) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        self.projection.list_open_after(after, limit).await
    }

    async fn list_recent(&self, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        self.projection.list_recent(limit).await
    }

    async fn put_runtime_process(
        &self,
        identity: &RuntimeProcessId,
    ) -> Result<(), AgentControlError> {
        self.projection.put_runtime_process(identity).await
    }

    async fn runtime_process(
        &self,
        agent: AgentId,
    ) -> Result<Option<RuntimeProcessId>, AgentControlError> {
        self.projection.runtime_process(agent).await
    }

    async fn clear_runtime_process(
        &self,
        identity: &RuntimeProcessId,
    ) -> Result<bool, AgentControlError> {
        self.projection.clear_runtime_process(identity).await
    }

    async fn record_recovery(
        &self,
        agent: AgentId,
        receipt: &AgentRecoveryReceipt,
    ) -> Result<AgentRunRecord, AgentControlError> {
        if let Some(run) = self.projection.get(agent).await? {
            self.ensure_started(&run).await?;
            self.supervisor
                .record_recovery(&runtime_agent(agent), format!("{:?}", receipt.decision))
                .await
                .map_err(runtime_error)?;
        }
        self.projection.record_recovery(agent, receipt).await
    }

    async fn compact_terminal(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentId>, AgentControlError> {
        self.projection.compact_terminal(now_ms, limit).await
    }

    async fn put_resource_lease(
        &self,
        lease: &AgentResourceLease,
    ) -> Result<(), AgentControlError> {
        self.projection.put_resource_lease(lease).await
    }

    async fn list_expired_resource_leases(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentResourceLease>, AgentControlError> {
        self.projection
            .list_expired_resource_leases(now_ms, limit)
            .await
    }

    async fn list_agent_resource_leases(
        &self,
        agent: AgentId,
        limit: usize,
    ) -> Result<Vec<AgentResourceLease>, AgentControlError> {
        self.projection
            .list_agent_resource_leases(agent, limit)
            .await
    }

    async fn delete_resource_lease(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError> {
        self.projection
            .delete_resource_lease(lease_key, expected_owner)
            .await
    }

    async fn append_message(
        &self,
        agent: AgentId,
        from: AgentId,
        delivery_id: uuid::Uuid,
        payload: &AgentMessagePayload,
        created_at_ms: i64,
    ) -> Result<AgentMessageRecord, AgentControlError> {
        let agent_run = runtime_agent(agent);
        let run = self
            .projection
            .get(agent)
            .await?
            .ok_or_else(|| AgentControlError::invalid("Agent projection row was not found"))?;
        self.supervisor
            .record_mailbox_delivery(
                &agent_run,
                delivery_id.to_string(),
                format!("{:?}", payload.kind),
                payload.correlation_id.map(|value| value.to_string()),
                AgentMailboxDelivery::Pending,
            )
            .await
            .map_err(runtime_error)?;
        let record = match self
            .projection
            .append_message(agent, from, delivery_id, payload, created_at_ms)
            .await
        {
            Ok(record) => record,
            Err(error) => {
                let _ = self
                    .supervisor
                    .record_mailbox_delivery(
                        &agent_run,
                        delivery_id.to_string(),
                        format!("{:?}", payload.kind),
                        payload.correlation_id.map(|value| value.to_string()),
                        AgentMailboxDelivery::Rejected,
                    )
                    .await;
                return Err(error);
            }
        };
        let _ = run;
        Ok(record)
    }

    async fn mark_message_delivery(
        &self,
        agent: AgentId,
        delivery_id: uuid::Uuid,
        delivery: AgentMessageDeliveryState,
    ) -> Result<AgentMessageRecord, AgentControlError> {
        let run = self
            .projection
            .get(agent)
            .await?
            .ok_or_else(|| AgentControlError::invalid("Agent projection row was not found"))?;
        self.supervisor
            .record_mailbox_delivery(
                &runtime_agent(agent),
                delivery_id.to_string(),
                "mailbox_delivery".into(),
                None,
                runtime_delivery(delivery),
            )
            .await
            .map_err(runtime_error)?;
        let _ = run;
        self.projection
            .mark_message_delivery(agent, delivery_id, delivery)
            .await
    }

    async fn record_terminal_receipt(
        &self,
        receipt: &AgentTerminalReceipt,
    ) -> Result<(), AgentControlError> {
        self.projection.record_terminal_receipt(receipt).await
    }

    async fn terminal_receipt(
        &self,
        agent: AgentId,
    ) -> Result<Option<AgentTerminalReceipt>, AgentControlError> {
        self.projection.terminal_receipt(agent).await
    }
}
