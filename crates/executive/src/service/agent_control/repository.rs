use async_trait::async_trait;
use fabric::{
    AgentBroadcastRef, AgentControlError, AgentId, AgentMessageDeliveryState, AgentMessagePayload,
    AgentRecoveryReceipt, AgentResult, AgentRunStatus, AgentSnapshot, AgentSpawnRequest,
    AgoraSpaceId, BroadcastEpoch, ProcessId, RuntimeResumability, VisibilityScope,
    WorkspaceCandidate,
};

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRunRecord {
    pub snapshot: AgentSnapshot,
    pub request: AgentSpawnRequest,
    pub request_hash: String,
    pub workspace_id: AgoraSpaceId,
    pub root_process_id: ProcessId,
    pub broadcast_refs: Vec<AgentBroadcastRef>,
    pub version: u64,
    pub retain_until_ms: i64,
    pub resumability: RuntimeResumability,
    pub recovery: Option<AgentRecoveryReceipt>,
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

    /// Return whether a durable, visibility-filtered broadcast item may be
    /// observed by this child. Exact space/epoch/content provenance is required
    /// in addition to the candidate's visibility scope.
    pub fn can_observe_broadcast(
        &self,
        epoch: BroadcastEpoch,
        candidate: &WorkspaceCandidate,
    ) -> bool {
        let referenced = self.broadcast_refs.iter().any(|reference| {
            reference.space == candidate.space
                && reference.epoch == epoch
                && reference.content_id == candidate.id
        });
        candidate.validate().is_ok()
            && referenced
            && match candidate.visibility {
                VisibilityScope::PrivateProcess { process } => {
                    process == self.snapshot.handle.process_id
                }
                VisibilityScope::AgentTree { root } => root == self.root_process_id,
                VisibilityScope::Session => true,
            }
    }
}

pub fn agent_workspace_id(agent: AgentId) -> AgoraSpaceId {
    AgoraSpaceId(format!("agent:{}", agent.0))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageRecord {
    pub delivery_id: uuid::Uuid,
    pub sequence: u64,
    pub from: AgentId,
    pub payload_ref: String,
    pub payload: AgentMessagePayload,
    pub delivery: AgentMessageDeliveryState,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentResourceLeaseKind {
    Admission,
    Mailbox,
    Execution,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResourceLease {
    pub lease_key: String,
    pub agent_id: AgentId,
    pub kind: AgentResourceLeaseKind,
    pub owner: String,
    pub expires_at_ms: i64,
    pub worktree_root: Option<String>,
    pub worktree_path: Option<String>,
    pub expected_head: Option<String>,
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

    async fn list_open(&self, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError>;

    async fn record_recovery(
        &self,
        agent: AgentId,
        receipt: &AgentRecoveryReceipt,
    ) -> Result<AgentRunRecord, AgentControlError>;

    async fn compact_terminal(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentId>, AgentControlError>;

    async fn put_resource_lease(&self, lease: &AgentResourceLease)
        -> Result<(), AgentControlError>;

    async fn list_expired_resource_leases(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentResourceLease>, AgentControlError>;

    async fn delete_resource_lease(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError>;

    async fn append_message(
        &self,
        agent: AgentId,
        from: AgentId,
        delivery_id: uuid::Uuid,
        payload: &AgentMessagePayload,
        created_at_ms: i64,
    ) -> Result<AgentMessageRecord, AgentControlError>;

    async fn mark_message_delivery(
        &self,
        agent: AgentId,
        delivery_id: uuid::Uuid,
        delivery: AgentMessageDeliveryState,
    ) -> Result<AgentMessageRecord, AgentControlError>;
}
