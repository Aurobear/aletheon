//! Agent identity validation projection and runtime capability translation.

use fabric::{AgentId, AgentProfileId, AgentRuntimeCapability, AgoraSpaceId, ProcessId};

pub(super) fn runtime_capability(value: &AgentRuntimeCapability) -> runtime::RuntimeCapability {
    match value {
        AgentRuntimeCapability::CodeRead => runtime::RuntimeCapability::CodeRead,
        AgentRuntimeCapability::CodeSearch => runtime::RuntimeCapability::CodeSearch,
        AgentRuntimeCapability::CodeEdit => runtime::RuntimeCapability::CodeEdit,
        AgentRuntimeCapability::Shell => runtime::RuntimeCapability::Shell,
        AgentRuntimeCapability::Test => runtime::RuntimeCapability::Test,
        AgentRuntimeCapability::Git => runtime::RuntimeCapability::Git,
        AgentRuntimeCapability::Diagnostics => runtime::RuntimeCapability::Diagnostics,
        AgentRuntimeCapability::Browser => runtime::RuntimeCapability::Browser,
        AgentRuntimeCapability::DeviceObserve => runtime::RuntimeCapability::DeviceObserve,
        AgentRuntimeCapability::DeviceCommand => runtime::RuntimeCapability::DeviceCommand,
        AgentRuntimeCapability::MemoryProposal => runtime::RuntimeCapability::MemoryProposal,
    }
}

pub(super) struct ValidatedAgentIdentity {
    pub(super) agent_id: AgentId,
    pub(super) root_process_id: Option<ProcessId>,
    pub(super) root_workspace_id: Option<AgoraSpaceId>,
    pub(super) depth: u16,
    pub(super) parent_profile: Option<AgentProfileId>,
}
