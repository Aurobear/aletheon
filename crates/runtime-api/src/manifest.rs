use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RuntimeCapability {
    CodeRead,
    CodeSearch,
    CodeEdit,
    Shell,
    Test,
    Git,
    Diagnostics,
    Browser,
    DeviceObserve,
    DeviceCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(PartialOrd, Ord)]
pub enum InteractionMode { OneShot, Resident, Steering, FollowUp }

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkspaceMode { Shared, IsolatedWorktree }

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolGovernance { Intercepted, Mediated, Observed, Opaque }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub id: String,
    pub aliases: Vec<String>,
    pub display_name: String,
    pub capabilities: BTreeSet<RuntimeCapability>,
    pub interaction_modes: BTreeSet<InteractionMode>,
    pub workspace_mode: WorkspaceMode,
    pub tool_governance: ToolGovernance,
}

impl RuntimeManifest {
    pub fn has(&self, cap: &RuntimeCapability) -> bool {
        self.capabilities.contains(cap)
    }
}
