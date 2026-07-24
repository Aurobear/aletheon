//! External runtime capability manifests and deterministic selection.

pub mod manifest;
pub mod selector;

pub use manifest::{
    InteractionMode, RuntimeCapability, RuntimeManifest, RuntimeResourceRequirements,
    ToolGovernance, WorkspaceMode, MAX_RUNTIME_STORAGE_BYTES, MAX_RUNTIME_STORAGE_ITEMS,
};
pub use selector::RuntimeSelector;
