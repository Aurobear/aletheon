//! Application-owned Agent command/query facade.

use std::sync::Arc;

use async_trait::async_trait;
use contracts::{
    AgentBroadcastRef, AgentControlError, AgentControlMessage, AgentControlPort, AgentHandle,
    AgentId, AgentListRequest, AgentSendRequest, AgentSnapshot, AgentSpawnIntent,
    AgentSpawnRequest, AgentWaitRequest, AgoraSpaceId, ProcessId,
};

#[derive(Debug, Clone)]
pub struct AgentCandidateProjectionContext {
    pub handle: AgentHandle,
    pub workspace_id: AgoraSpaceId,
    pub root_workspace_id: AgoraSpaceId,
    pub root_process_id: ProcessId,
    pub broadcast_refs: Vec<AgentBroadcastRef>,
}

#[derive(Clone)]
pub struct AgentService {
    control: Arc<dyn AgentControlPort>,
}

impl AgentService {
    pub fn new(control: Arc<dyn AgentControlPort>) -> Self {
        Self { control }
    }
}

#[async_trait]
impl AgentControlPort for AgentService {
    async fn spawn_intent(
        &self,
        intent: AgentSpawnIntent,
    ) -> Result<AgentHandle, AgentControlError> {
        self.control.spawn_intent(intent).await
    }

    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        self.control.spawn(request).await
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        self.control.wait(request).await
    }

    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        self.control.send(request).await
    }

    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.control.cancel(caller_root_agent_id, agent_id).await
    }

    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.control.inspect(caller_root_agent_id, agent_id).await
    }

    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        self.control.list(request).await
    }
}
