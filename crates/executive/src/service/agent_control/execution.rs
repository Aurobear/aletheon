use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentHandle, AgentId, AgentResult, AgentRunStatus,
    AgentSpawnRequest, AgoraSpaceId, OperationId, ProcessId, RuntimeId,
};
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::core::sub_agent::{SubAgentExecutionContext, SubAgentRuntime};

use super::context_fork::AgentContextProjection;
use super::mailbox::AgentRuntimeInbox;

#[derive(Debug, Clone)]
pub enum AgentRuntimeEvent {
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
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentRuntimeEvent);
}

#[derive(Debug, Default)]
pub struct NoopAgentEventSink;

#[async_trait]
impl AgentEventSink for NoopAgentEventSink {
    async fn emit(&self, _event: AgentRuntimeEvent) {}
}

#[derive(Debug, Clone)]
pub struct AgentRuntimeInput {
    pub request: AgentSpawnRequest,
    pub handle: AgentHandle,
    pub workspace_id: AgoraSpaceId,
    /// Root conscious workspace. Child-private candidates never use this
    /// space; explicitly exportable candidates are admitted here for a later
    /// C01 selection cycle.
    pub root_workspace_id: AgoraSpaceId,
    pub root_process_id: ProcessId,
    pub context: AgentContextProjection,
    pub inbox: AgentRuntimeInbox,
    pub cancellation: CancellationToken,
}

#[async_trait]
pub trait AgentRuntimeLauncher: Send + Sync {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError>;
}

#[derive(Default)]
pub struct AgentRuntimeRegistry {
    runtimes: RwLock<HashMap<RuntimeId, Arc<dyn AgentRuntimeLauncher>>>,
}

impl AgentRuntimeRegistry {
    pub fn register(
        &self,
        id: RuntimeId,
        launcher: Arc<dyn AgentRuntimeLauncher>,
    ) -> Result<(), AgentControlError> {
        if id.0.trim().is_empty() {
            return Err(AgentControlError::invalid("runtime id must not be empty"));
        }
        let mut runtimes = self.runtimes.write();
        if runtimes.contains_key(&id) {
            return Err(AgentControlError {
                kind: AgentControlErrorKind::Conflict,
                message: format!("runtime already registered: {}", id.0),
            });
        }
        runtimes.insert(id, launcher);
        Ok(())
    }

    pub fn resolve(
        &self,
        id: &RuntimeId,
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError> {
        self.runtimes
            .read()
            .get(id)
            .cloned()
            .ok_or_else(|| AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message: format!("runtime is not registered: {}", id.0),
            })
    }
}

pub struct CompatibilityRuntimeLauncher {
    runtime: Arc<dyn SubAgentRuntime>,
}

impl CompatibilityRuntimeLauncher {
    pub fn new(runtime: Arc<dyn SubAgentRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl AgentRuntimeLauncher for CompatibilityRuntimeLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        events
            .emit(AgentRuntimeEvent::Started {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
            })
            .await;
        let context = SubAgentExecutionContext {
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            session_id: input.handle.root_agent_id.0.to_string(),
            working_dir: std::env::current_dir().unwrap_or_default(),
        };
        let output = self
            .runtime
            .run_in_context(&input.request.task, input.cancellation, context)
            .await
            .map_err(|message| AgentControlError {
                kind: AgentControlErrorKind::Runtime,
                message,
            })?;
        let result = AgentResult {
            output,
            usage: fabric::AttemptUsage::default(),
            evidence: vec![],
            artifacts: vec![],
        };
        result.validate()?;
        events
            .emit(AgentRuntimeEvent::Terminal {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
                status: AgentRunStatus::Succeeded,
                result: Some(result.clone()),
            })
            .await;
        Ok(result)
    }
}
