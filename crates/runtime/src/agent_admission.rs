//! Runtime-owned Agent admission contracts (RA-05).
//!
//! The admission port carries topology, storage and budget intent into the
//! host adapter. Runtime owns the typed contract and identity mint; concrete
//! policy/budget implementations remain injectable adapters.

use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentId, AgentProfileId, AgentSpawnRequest,
    AttemptUsage, BudgetTransferReceipt,
};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentStorageRequest {
    pub bytes: u64,
    pub items: u64,
}

pub struct AgentAdmissionRequest<'a> {
    pub agent_id: AgentId,
    pub spawn: &'a AgentSpawnRequest,
    pub depth: u16,
    pub parent_profile: Option<&'a AgentProfileId>,
    pub storage: AgentStorageRequest,
}

impl<'a> AgentAdmissionRequest<'a> {
    /// Fixture/convenience constructor. Production callers should use the
    /// Runtime receipt supplied by `RuntimeAgentSupervisor` and
    /// `new_for_agent`, so adapters cannot mint a second identity.
    pub fn new(
        spawn: &'a AgentSpawnRequest,
        depth: u16,
        parent_profile: Option<&'a AgentProfileId>,
        storage: AgentStorageRequest,
    ) -> Self {
        Self::new_for_agent(
            AgentId(crate::mint_agent_run_uuid()),
            spawn,
            depth,
            parent_profile,
            storage,
        )
    }

    pub fn new_for_agent(
        agent_id: AgentId,
        spawn: &'a AgentSpawnRequest,
        depth: u16,
        parent_profile: Option<&'a AgentProfileId>,
        storage: AgentStorageRequest,
    ) -> Self {
        Self {
            agent_id,
            spawn,
            depth,
            parent_profile,
            storage,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentAdmissionMetrics {
    pub roots: usize,
    pub resident_agents: usize,
    pub queued_agents: usize,
    pub running_agents: usize,
    pub resident_idle_agents: usize,
    pub reserved_storage_bytes: u64,
    pub reserved_storage_items: u64,
}

#[async_trait]
pub trait AgentAdmissionLease: Send {
    async fn mark_running(&mut self) -> Result<(), AgentControlError>;
    async fn settle(&mut self, usage: &AttemptUsage) -> Result<(), AgentControlError>;
    async fn revoke(&mut self) -> Result<(), AgentControlError>;
    async fn transfer_remaining_to(
        &mut self,
        parent: AgentId,
        usage: &AttemptUsage,
    ) -> Result<BudgetTransferReceipt, AgentControlError>;
}

#[async_trait]
pub trait AgentAdmissionPort: Send + Sync {
    async fn reserve(
        &self,
        request: AgentAdmissionRequest<'_>,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError>;

    fn metrics(&self) -> AgentAdmissionMetrics;
}

pub fn constrain_cognitive_workspace(
    request: &mut AgentSpawnRequest,
) -> Result<(), AgentControlError> {
    let Some(binding) = request.cognitive_binding.as_ref() else {
        return Ok(());
    };
    let role = binding.role;
    let scope = binding.workspace_scope.clone();
    if role.can_write_workspace() && scope.is_empty() {
        return Err(forbidden(
            "writable cognitive role has an empty workspace scope",
        ));
    }
    if !role.can_write_workspace() && !scope.is_empty() {
        return Err(forbidden(
            "read-only cognitive role received a writable workspace scope",
        ));
    }
    let workspace = request
        .trusted_workspace
        .clone()
        .ok_or_else(|| forbidden("cognitive Agent spawn has no trusted workspace authority"))?;
    let effective_scope = if role.can_write_workspace() {
        scope.as_slice()
    } else {
        &[]
    };
    request.trusted_workspace = Some(
        workspace
            .narrow_to_declared_paths(effective_scope)
            .map_err(AgentControlError::invalid)?,
    );
    Ok(())
}

fn forbidden(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Forbidden,
        message: message.into(),
    }
}

/// Binds a Runtime-admitted child to its canonical cognitive task before the
/// host launches any external process.
#[async_trait::async_trait]
pub trait CognitiveTaskAdmissionPort: Send + Sync {
    async fn bind_before_launch(
        &self,
        binding: ::contracts::cognitive_workflow::CognitiveTaskRuntimeBinding,
        allocated_process: ::contracts::ProcessId,
    ) -> Result<(), ::contracts::AgentControlError>;
}
