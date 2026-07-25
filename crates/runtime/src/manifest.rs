use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use fabric::{
    AgentInteractionMode as InteractionMode, AgentRuntimeCapability as RuntimeCapability,
    AgentTaskEncoding as TaskEncoding, AgentWorkspaceMode as WorkspaceMode,
};

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
    pub workspace_modes: BTreeSet<WorkspaceMode>,
    pub task_encodings: BTreeSet<TaskEncoding>,
    pub tool_governance: ToolGovernance,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub max_context_tokens: Option<u64>,
    #[serde(default)]
    pub resource_requirements: RuntimeResourceRequirements,
}

impl RuntimeManifest {
    pub fn has(&self, cap: &RuntimeCapability) -> bool {
        self.capabilities.contains(cap)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("runtime id must not be empty".into());
        }
        if self.interaction_modes.is_empty() {
            return Err("runtime interaction modes must not be empty".into());
        }
        if self.workspace_modes.is_empty() {
            return Err("runtime workspace modes must not be empty".into());
        }
        if self.task_encodings.is_empty() {
            return Err("runtime task encodings must not be empty".into());
        }
        if self.max_context_tokens == Some(0) {
            return Err("runtime context limit must be nonzero".into());
        }
        self.resource_requirements.validate()?;
        Ok(())
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

    #[test]
    fn selectable_manifest_round_trips_generic_constraints() {
        let manifest = RuntimeManifest {
            id: "analysis-a".into(),
            aliases: vec!["analysis".into()],
            display_name: "Analysis A".into(),
            capabilities: BTreeSet::from([
                RuntimeCapability::CodeRead,
                RuntimeCapability::CodeSearch,
            ]),
            interaction_modes: BTreeSet::from([InteractionMode::Resident]),
            workspace_modes: BTreeSet::from([WorkspaceMode::SharedReadOnly]),
            task_encodings: BTreeSet::from([TaskEncoding::NaturalLanguage]),
            tool_governance: ToolGovernance::Observed,
            priority: 10,
            max_context_tokens: Some(1_000_000),
            resource_requirements: Default::default(),
        };
        manifest.validate().unwrap();
        let encoded = serde_json::to_value(&manifest).unwrap();
        assert_eq!(encoded["priority"], 10);
        assert_eq!(
            serde_json::from_value::<RuntimeManifest>(encoded)
                .unwrap()
                .workspace_modes,
            manifest.workspace_modes
        );
    }

    #[test]
    fn selectable_manifest_requires_an_input_encoding() {
        let mut manifest = RuntimeManifest {
            id: "analysis-a".into(),
            aliases: vec![],
            display_name: "Analysis A".into(),
            capabilities: BTreeSet::new(),
            interaction_modes: BTreeSet::from([InteractionMode::Resident]),
            workspace_modes: BTreeSet::from([WorkspaceMode::SharedReadOnly]),
            task_encodings: BTreeSet::from([TaskEncoding::NaturalLanguage]),
            tool_governance: ToolGovernance::Observed,
            priority: 0,
            max_context_tokens: None,
            resource_requirements: Default::default(),
        };
        manifest.task_encodings.clear();
        assert_eq!(
            manifest.validate().unwrap_err(),
            "runtime task encodings must not be empty"
        );
    }
}
