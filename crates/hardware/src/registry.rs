//! Registry of embodiment providers keyed by canonical device identity.

use std::{collections::HashMap, sync::Arc};

use fabric::DeviceId;

use crate::EmbodimentProvider;

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<DeviceId, Arc<dyn EmbodimentProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, device: DeviceId, provider: Arc<dyn EmbodimentProvider>) {
        self.providers.insert(device, provider);
    }

    pub fn provider(&self, device: &DeviceId) -> Option<Arc<dyn EmbodimentProvider>> {
        self.providers.get(device).cloned()
    }

    pub fn device_count(&self) -> usize {
        self.providers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_fails_closed() {
        let registry = ProviderRegistry::new();
        assert!(registry.provider(&DeviceId("bot".into())).is_none());
        assert_eq!(registry.device_count(), 0);
    }
}
