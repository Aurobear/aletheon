use runtime_api::manifest::RuntimeCapability;
use runtime_api::CapabilityRuntime;
use std::sync::Arc;
use crate::registry::RuntimeRegistry;

pub enum RuntimeSelector {
    Auto,
    Alias(String),
    RequiredCapabilities(Vec<RuntimeCapability>),
}

impl RuntimeSelector {
    pub fn resolve(
        &self,
        registry: &RuntimeRegistry,
        required: &[RuntimeCapability],
    ) -> Option<Arc<dyn CapabilityRuntime>> {
        match self {
            RuntimeSelector::Auto => {
                registry.list_ids().first()
                    .and_then(|id| registry.get(id))
                    .map(|r| Arc::clone(r))
            }
            RuntimeSelector::Alias(name) => {
                registry.get(name).map(|r| Arc::clone(r))
            }
            RuntimeSelector::RequiredCapabilities(caps) => {
                let mut all = caps.clone();
                all.extend_from_slice(required);
                registry.list_ids().iter()
                    .filter_map(|id| registry.get(id))
                    .find(|r| all.iter().all(|c| r.manifest().has(c)))
                    .map(|r| Arc::clone(r))
            }
        }
    }
}

/// Convenience: select a runtime by selector.
pub fn select(
    registry: &RuntimeRegistry,
    selector: &RuntimeSelector,
) -> Option<Arc<dyn CapabilityRuntime>> {
    selector.resolve(registry, &[])
}
