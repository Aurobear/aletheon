//! External runtime capability manifests and deterministic selection.

pub mod manifest;
pub mod selector;

pub use manifest::{
    InteractionMode, RuntimeCapability, RuntimeManifest, ToolGovernance, WorkspaceMode,
};
pub use selector::RuntimeSelector;
