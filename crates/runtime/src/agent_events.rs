//! Runtime-owned Agent execution event contract (RA-05).
//!
//! Host execution adapters may emit these observations, but Runtime owns the
//! typed event shape and the lifecycle writer that consumes it. Projection and
//! memory sinks remain host adapters.

use ::contracts::{AgentId, AgentResult, AgentRunStatus, OperationId, ProcessId};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub enum AgentRuntimeEvent {
    CapabilityAttenuated {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        report: ::contracts::AgentAttenuationReport,
    },
    Started {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
    },
    Progress {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        summary: String,
    },
    Tool {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        name: String,
        is_error: bool,
    },
    Terminal {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        status: AgentRunStatus,
        result: Option<AgentResult>,
    },
}

#[async_trait]
pub trait AgentRuntimeEventSink: Send + Sync {
    async fn emit(&self, event: AgentRuntimeEvent);
}
