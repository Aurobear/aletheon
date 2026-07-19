//! Backend registry — maps host platforms to capability providers.

use platform_api::HostCapabilityManifest;
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of host capability backends keyed by platform name.
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn crate::selector::Backend>>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, backend: Arc<dyn crate::selector::Backend>) {
        self.backends.insert(name.into(), backend);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn crate::selector::Backend>> {
        self.backends.get(name)
    }

    pub fn probe_all(&self) -> Vec<(String, HostCapabilityManifest)> {
        self.backends
            .iter()
            .map(|(name, backend)| (name.clone(), backend.probe()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::select_backend;

    #[test]
    fn registry_empty_by_default() {
        assert!(BackendRegistry::new().get("linux").is_none());
    }

    #[test]
    fn registry_holds_backend_and_probes() {
        let mut reg = BackendRegistry::new();
        let backend = Arc::from(select_backend());
        reg.register("stub", backend);
        let manifests = reg.probe_all();
        assert_eq!(manifests.len(), 1);
    }
}
