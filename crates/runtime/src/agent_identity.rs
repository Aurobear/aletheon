//! Runtime-owned identity validation and capability translation for agents.

use ::contracts::{AgentId, AgentProfileId, AgentRuntimeCapability, AgoraSpaceId, ProcessId};

use crate::{Generation, RuntimeCapability};

/// Translate the wire-level capability manifest into Runtime semantics.
pub fn runtime_capability(value: &AgentRuntimeCapability) -> RuntimeCapability {
    match value {
        AgentRuntimeCapability::CodeRead => RuntimeCapability::CodeRead,
        AgentRuntimeCapability::CodeSearch => RuntimeCapability::CodeSearch,
        AgentRuntimeCapability::CodeEdit => RuntimeCapability::CodeEdit,
        AgentRuntimeCapability::Shell => RuntimeCapability::Shell,
        AgentRuntimeCapability::Test => RuntimeCapability::Test,
        AgentRuntimeCapability::Git => RuntimeCapability::Git,
        AgentRuntimeCapability::Diagnostics => RuntimeCapability::Diagnostics,
        AgentRuntimeCapability::Browser => RuntimeCapability::Browser,
        AgentRuntimeCapability::DeviceObserve => RuntimeCapability::DeviceObserve,
        AgentRuntimeCapability::DeviceCommand => RuntimeCapability::DeviceCommand,
        AgentRuntimeCapability::MemoryProposal => RuntimeCapability::MemoryProposal,
    }
}

/// Identity receipt produced by Runtime admission and consumed by host orchestration.
pub struct ValidatedAgentIdentity {
    pub agent_id: AgentId,
    /// Generation assigned by Runtime for this identity, when the request
    /// entered through the Runtime admission path. Keep this receipt on the
    /// validated identity instead of mirroring it in an Executive map: the
    /// Runtime supervisor is the sole generation authority.
    pub runtime_generation: Option<Generation>,
    pub root_process_id: Option<ProcessId>,
    pub root_workspace_id: Option<AgoraSpaceId>,
    pub depth: u16,
    pub parent_profile: Option<AgentProfileId>,
}
