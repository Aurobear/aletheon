//! Runtime-owned AgentStream adapter (RA-05).
//!
//! Host execution surfaces may still expose a richer event sink for memory,
//! projections, or compatibility UI.  This adapter makes the Runtime
//! AgentStream the first durable lifecycle boundary and only forwards the
//! observation after the canonical Runtime fence accepts it.

use std::sync::Arc;

use ::contracts::{AgentResult, AgentRunStatus};
use async_trait::async_trait;

use crate::{AgentRunId, AgentRuntimeEvent, AgentRuntimeEventSink, RuntimeAgentSupervisor};

/// One-way adapter from a host event sink to the Runtime-owned AgentStream.
///
/// The host may continue to project a compatibility event after this adapter,
/// but it cannot mint a second AgentRun identity or publish a terminal before
/// Runtime accepts the durable terminal fence.
pub struct RuntimeAgentStreamAdapter {
    downstream: Arc<dyn AgentRuntimeEventSink>,
    supervisor: Arc<RuntimeAgentSupervisor>,
    agent_run: AgentRunId,
}

impl RuntimeAgentStreamAdapter {
    pub fn new(
        downstream: Arc<dyn AgentRuntimeEventSink>,
        supervisor: Arc<RuntimeAgentSupervisor>,
        agent_run: AgentRunId,
    ) -> Self {
        Self {
            downstream,
            supervisor,
            agent_run,
        }
    }
}

#[async_trait]
impl AgentRuntimeEventSink for RuntimeAgentStreamAdapter {
    async fn emit(&self, event: AgentRuntimeEvent) {
        // Preserve host lifecycle/mailbox observations in the Runtime-owned
        // AgentStream before forwarding the richer compatibility event to
        // SQL/memory projections. The payload is intentionally a typed kind
        // and correlation only; public AgentResult content remains a separate
        // host projection schema.
        let observation = match &event {
            AgentRuntimeEvent::CapabilityAttenuated { operation_id, .. } => {
                Some(("capability_attenuated", operation_id))
            }
            AgentRuntimeEvent::Started { operation_id, .. } => Some(("started", operation_id)),
            AgentRuntimeEvent::Progress { operation_id, .. } => Some(("progress", operation_id)),
            AgentRuntimeEvent::Tool { operation_id, .. } => Some(("tool", operation_id)),
            AgentRuntimeEvent::Terminal { .. } => None,
        };
        if let Some((kind, operation_id)) = observation {
            if let Err(error) = self
                .supervisor
                .record_message(
                    &self.agent_run,
                    kind.into(),
                    Some(operation_id.0.to_string()),
                )
                .await
            {
                tracing::warn!(
                    agent_run = %self.agent_run.0,
                    %error,
                    "Runtime Agent observation append rejected"
                );
            }
        }

        let terminal = match &event {
            AgentRuntimeEvent::Terminal { status, result, .. } => match status {
                AgentRunStatus::Succeeded => Some(crate::TurnTerminal::Completed),
                AgentRunStatus::Cancelled | AgentRunStatus::Interrupted => {
                    Some(crate::TurnTerminal::Interrupted)
                }
                AgentRunStatus::Failed => Some(crate::TurnTerminal::Failed {
                    message: result
                        .as_ref()
                        .map(|result: &AgentResult| result.output.chars().take(1024).collect())
                        .unwrap_or_else(|| "Agent runtime failed".into()),
                }),
                AgentRunStatus::Queued | AgentRunStatus::Running | AgentRunStatus::Waiting => None,
            },
            _ => None,
        };
        if let Some(terminal) = terminal {
            // Runtime is the terminal authority. Do not publish a legacy
            // terminal observation first: if the canonical AgentStream
            // receipt is rejected, downstream compatibility state must not
            // claim a settled run that Runtime could not durably fence.
            if let Err(error) = self
                .supervisor
                .record_settled(&self.agent_run, terminal)
                .await
            {
                tracing::warn!(
                    agent_run = %self.agent_run.0,
                    %error,
                    "Runtime Agent terminal receipt append rejected"
                );
                return;
            }
        }
        self.downstream.emit(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{AgentId, OperationId, ProcessId};
    use std::sync::Mutex;

    struct RecordingSink(Arc<Mutex<Vec<AgentRuntimeEvent>>>);

    #[async_trait]
    impl AgentRuntimeEventSink for RecordingSink {
        async fn emit(&self, event: AgentRuntimeEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn adapter_fences_observation_before_forwarding() {
        let supervisor = Arc::new(RuntimeAgentSupervisor::new(
            crate::DelegateBackendRegistry::new(),
        ));
        let agent_run = AgentRunId("run-adapter-test".into());
        supervisor
            .record_started(
                crate::SessionId("session-adapter-test".into()),
                agent_run.clone(),
                crate::Generation(1),
                Some("native".into()),
            )
            .await
            .unwrap();

        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let adapter = RuntimeAgentStreamAdapter::new(
            Arc::new(RecordingSink(forwarded.clone())),
            supervisor,
            agent_run,
        );
        adapter
            .emit(AgentRuntimeEvent::Started {
                agent_id: AgentId::new(),
                process_id: ProcessId::new(),
                operation_id: OperationId::new(),
            })
            .await;

        assert_eq!(forwarded.lock().unwrap().len(), 1);
    }
}
