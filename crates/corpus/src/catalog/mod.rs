//! Corpus-owned immutable extension catalog.

use std::collections::BTreeMap;

use fabric::{ExtensionDescriptor, ExtensionId, ExtensionSnapshot};

use crate::service::CorpusError;

/// Deterministic metadata index. Constructing a catalog never activates entries.
#[derive(Debug, Clone, Default)]
pub struct ExtensionCatalog {
    entries: BTreeMap<ExtensionId, ExtensionDescriptor>,
    executable_capabilities: BTreeMap<String, ExtensionId>,
}

impl ExtensionCatalog {
    pub fn new(
        descriptors: impl IntoIterator<Item = ExtensionDescriptor>,
    ) -> Result<Self, CorpusError> {
        let mut catalog = Self::default();
        for descriptor in descriptors {
            catalog.register(descriptor)?;
        }
        Ok(catalog)
    }

    pub fn register(&mut self, descriptor: ExtensionDescriptor) -> Result<(), CorpusError> {
        let id = descriptor.id.clone();
        if self.entries.contains_key(&id) {
            return Err(CorpusError::DuplicateExtension(id.as_str().to_string()));
        }
        if descriptor.capabilities.is_empty() {
            return Err(CorpusError::InvalidDescriptor(format!(
                "{} declares no capabilities",
                id.as_str()
            )));
        }
        if descriptor.is_executable() {
            for capability in &descriptor.capabilities {
                if let Some(existing) = self.executable_capabilities.get(&capability.0) {
                    return Err(CorpusError::ConflictingCapability {
                        capability: capability.0.clone(),
                        first: existing.as_str().to_string(),
                        second: id.as_str().to_string(),
                    });
                }
            }
        }
        for capability in &descriptor.capabilities {
            if descriptor.is_executable() {
                self.executable_capabilities
                    .insert(capability.0.clone(), id.clone());
            }
        }
        self.entries.insert(id, descriptor);
        Ok(())
    }

    pub(crate) fn entries(&self) -> &BTreeMap<ExtensionId, ExtensionDescriptor> {
        &self.entries
    }
}

impl fabric::ExtensionCatalog for ExtensionCatalog {
    fn snapshot(&self) -> ExtensionSnapshot {
        ExtensionSnapshot {
            entries: self.entries.values().cloned().collect(),
        }
    }
}
