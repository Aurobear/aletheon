//! E1 extension registration descriptor + owner port (Agent Kernel V2).
//!
//! Narrow extension seam: a registration descriptor enumerates an extension's
//! stable identity, feature flags and worker/credential references — without
//! rich types, without a second composition root, and without carrying any
//! concrete adapter.  The core binary constructs with **zero extensions**
//! (empty registry is valid).  Rich types stay in their owner crates; nothing
//! here goes into `contracts`.

use serde::{Deserialize, Serialize};

/// Stable extension identity (owner-local, not a canonical core ID).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExtensionId(pub String);

/// Feature/configuration flags an extension declares.  Owner-local; the core
/// only records them, it does not interpret them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionFlags {
    pub enabled: bool,
    /// Optional installed-state reference (e.g. spool path, OAuth ref).  The
    /// core never reads these; the extension owner does.
    pub installed_reference: Option<String>,
}

/// Registration descriptor for one preserved extension.  No rich types, no
/// concrete adapter, no secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRegistration {
    pub id: ExtensionId,
    pub flags: ExtensionFlags,
}

/// Owner port for extension registration.  The composition root registers
/// zero or more extensions; the core binary must construct with an empty set.
pub trait ExtensionPort: Send + Sync {
    fn register(&mut self, registration: ExtensionRegistration);
    fn registered(&self) -> Vec<ExtensionId>;
}

/// In-memory extension registry.  Empty by default — the core binary
/// constructs with zero extensions (E1 requirement).
#[derive(Debug, Default)]
pub struct InMemoryExtensionRegistry {
    extensions: std::collections::HashMap<ExtensionId, ExtensionFlags>,
}

impl InMemoryExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExtensionPort for InMemoryExtensionRegistry {
    fn register(&mut self, registration: ExtensionRegistration) {
        self.extensions
            .insert(registration.id.clone(), registration.flags);
    }

    fn registered(&self) -> Vec<ExtensionId> {
        self.extensions.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_binary_constructs_with_zero_extensions() {
        let registry = InMemoryExtensionRegistry::new();
        assert!(registry.registered().is_empty());
    }

    #[test]
    fn registers_and_lists_extension() {
        let mut registry = InMemoryExtensionRegistry::new();
        registry.register(ExtensionRegistration {
            id: ExtensionId("gbrain".into()),
            flags: ExtensionFlags {
                enabled: true,
                installed_reference: Some("spool:gbrain".into()),
            },
        });
        assert_eq!(registry.registered(), vec![ExtensionId("gbrain".into())]);
    }
}
