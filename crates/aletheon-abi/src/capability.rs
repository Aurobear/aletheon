//! Capability-based security — like Linux capabilities.
//!
//! Subsystems declare capabilities at init time. The runtime checks
//! capabilities before allowing privileged operations.

use serde::{Deserialize, Serialize};

/// Permission level for an operation.
///
/// Permission level for runtime operations.
///
/// Tool-level (L0-L3) maps to these ABI-level semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Read-only operations (no side effects).
    #[default]
    ReadOnly = 0,
    /// Write within sandbox boundaries.
    SandboxWrite = 1,
    /// System-level changes (files, processes, config).
    SystemChange = 2,
    /// Destructive / irreversible operations.
    Destructive = 3,
    /// Self-modification (MetaRuntime only).
    SelfModify = 4,
}

/// A capability declaration — what a subsystem can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name (e.g., "shell.execute", "memory.write", "self.mutate").
    pub name: String,
    /// Maximum permission level this capability grants.
    pub level: PermissionLevel,
    /// Human-readable description.
    pub description: String,
}

impl Capability {
    pub fn new(name: impl Into<String>, level: PermissionLevel, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level,
            description: description.into(),
        }
    }
}

/// Set of capabilities — attached to a Context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitySet {
    capabilities: Vec<Capability>,
}

impl CapabilitySet {
    pub fn new() -> Self {
        Self { capabilities: Vec::new() }
    }

    pub fn with(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }

    pub fn add(&mut self, cap: Capability) {
        self.capabilities.push(cap);
    }

    /// Check if the set includes a capability with at least the given level.
    pub fn has(&self, name: &str, min_level: PermissionLevel) -> bool {
        self.capabilities
            .iter()
            .any(|c| c.name == name && c.level >= min_level)
    }

    /// Check if the set includes a capability with the given name.
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c.name == name)
    }

    /// Get the maximum permission level across all capabilities.
    pub fn max_level(&self) -> PermissionLevel {
        self.capabilities
            .iter()
            .map(|c| c.level)
            .max()
            .unwrap_or(PermissionLevel::ReadOnly)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    pub fn len(&self) -> usize {
        self.capabilities.len()
    }
}
