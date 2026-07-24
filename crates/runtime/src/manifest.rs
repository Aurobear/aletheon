use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_RUNTIME_STORAGE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_RUNTIME_STORAGE_ITEMS: u64 = 1_000_000;

/// Bounded resources requested by a runtime manifest.  Admission remains the
/// authorization owner and may reject or reserve less than this declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeResourceRequirements {
    pub storage_bytes: u64,
    pub storage_items: u64,
}

impl RuntimeResourceRequirements {
    pub fn validate(self) -> Result<Self, String> {
        if self.storage_bytes > MAX_RUNTIME_STORAGE_BYTES {
            return Err(format!(
                "runtime storage byte request exceeds {MAX_RUNTIME_STORAGE_BYTES}"
            ));
        }
        if self.storage_items > MAX_RUNTIME_STORAGE_ITEMS {
            return Err(format!(
                "runtime storage item request exceeds {MAX_RUNTIME_STORAGE_ITEMS}"
            ));
        }
        Ok(self)
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum InteractionMode {
    OneShot,
    Resident,
    Steering,
    FollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkspaceMode {
    Shared,
    IsolatedWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolGovernance {
    Intercepted,
    Mediated,
    Observed,
    Opaque,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub id: String,
    pub aliases: Vec<String>,
    pub display_name: String,
    pub capabilities: BTreeSet<RuntimeCapability>,
    pub interaction_modes: BTreeSet<InteractionMode>,
    pub workspace_mode: WorkspaceMode,
    pub tool_governance: ToolGovernance,
    #[serde(default)]
    pub resource_requirements: RuntimeResourceRequirements,
}

impl RuntimeManifest {
    pub fn has(&self, cap: &RuntimeCapability) -> bool {
        self.capabilities.contains(cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_requirements_default_to_no_reservation() {
        assert_eq!(
            RuntimeResourceRequirements::default().validate().unwrap(),
            RuntimeResourceRequirements::default()
        );
    }

    #[test]
    fn resource_requirements_reject_system_maximum_overflow() {
        assert!(RuntimeResourceRequirements {
            storage_bytes: MAX_RUNTIME_STORAGE_BYTES + 1,
            storage_items: 0,
        }
        .validate()
        .is_err());
        assert!(RuntimeResourceRequirements {
            storage_bytes: 0,
            storage_items: MAX_RUNTIME_STORAGE_ITEMS + 1,
        }
        .validate()
        .is_err());
    }
}
