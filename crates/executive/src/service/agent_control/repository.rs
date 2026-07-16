use async_trait::async_trait;
use fabric::{
    AgentControlError, AgentId, AgentResult, AgentRunStatus, AgentSnapshot, AgentSpawnRequest,
};

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRunRecord {
    pub snapshot: AgentSnapshot,
    pub request: AgentSpawnRequest,
    pub request_hash: String,
    pub version: u64,
    pub retain_until_ms: i64,
}

impl AgentRunRecord {
    pub fn agent_id(&self) -> AgentId {
        self.snapshot.handle.agent_id
    }

    pub fn root_agent_id(&self) -> AgentId {
        self.snapshot.handle.root_agent_id
    }

    pub fn status(&self) -> AgentRunStatus {
        self.snapshot.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageRecord {
    pub sequence: u64,
    pub content_hash: String,
}

#[async_trait]
pub trait AgentRunRepository: Send + Sync {
    async fn create(&self, run: &AgentRunRecord) -> Result<(), AgentControlError>;

    async fn transition(
        &self,
        agent: AgentId,
        expected: AgentRunStatus,
        next: AgentRunStatus,
        result: Option<AgentResult>,
        error: Option<String>,
        now_ms: i64,
    ) -> Result<AgentRunRecord, AgentControlError>;

    async fn get(&self, agent: AgentId) -> Result<Option<AgentRunRecord>, AgentControlError>;

    async fn list_root(
        &self,
        root: AgentId,
        status: Option<AgentRunStatus>,
        limit: usize,
    ) -> Result<Vec<AgentRunRecord>, AgentControlError>;

    async fn append_message(
        &self,
        agent: AgentId,
        from: AgentId,
        content: &str,
        created_at_ms: i64,
    ) -> Result<AgentMessageRecord, AgentControlError>;
}
