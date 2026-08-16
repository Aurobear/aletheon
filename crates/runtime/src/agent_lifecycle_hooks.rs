//! Runtime-owned lifecycle hook contract and context projection.

use ::contracts::{AgentHandle, WorkspacePolicy};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLifecyclePoint {
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLifecycleObservation {
    pub point: AgentLifecyclePoint,
    pub root_agent_id: String,
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub operation_id: String,
    pub task: String,
    pub status: String,
    pub workspace_root: Option<String>,
}

/// Host-independent sink for lifecycle observations emitted by a supervised
/// Agent. Concrete sinks (for example Corpus) remain outside Runtime.
#[async_trait]
pub trait AgentLifecycleHookSink: Send + Sync {
    async fn emit(&self, observation: AgentLifecycleObservation);
}

/// No-op hook used when no optional lifecycle integration is configured.
pub struct NoopAgentLifecycleHookSink;

#[async_trait]
impl AgentLifecycleHookSink for NoopAgentLifecycleHookSink {
    async fn emit(&self, _observation: AgentLifecycleObservation) {}
}

/// Build the stable, bounded hook context from the Runtime Agent identity
/// receipt and host-provided task/workspace values.
pub fn agent_lifecycle_hook_context(
    point: AgentLifecyclePoint,
    handle: &AgentHandle,
    task: &str,
    workspace: Option<&WorkspacePolicy>,
    status: &str,
) -> AgentLifecycleObservation {
    AgentLifecycleObservation {
        point,
        root_agent_id: handle.root_agent_id.0.to_string(),
        agent_id: handle.agent_id.0.to_string(),
        parent_agent_id: handle.parent_agent_id.map(|id| id.0.to_string()),
        operation_id: handle.operation_id.0.to_string(),
        task: task.to_owned(),
        status: status.into(),
        workspace_root: workspace.map(|value| value.cwd().to_string_lossy().into_owned()),
    }
}
