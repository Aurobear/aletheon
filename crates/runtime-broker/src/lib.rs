//! Runtime Broker — selects the right CapabilityRuntime by alias or
//! required capabilities, with health checks and fallback policy.

pub mod registry;
pub mod selector;

pub use registry::RuntimeRegistry;
pub use selector::{RuntimeSelector, select};

use runtime_api::manifest::RuntimeCapability;
use runtime_api::CapabilityRuntime;
use std::sync::Arc;

/// Resolve a selector to a concrete runtime, with fallback.
pub fn resolve(
    registry: &RuntimeRegistry,
    selector: &RuntimeSelector,
    required: &[RuntimeCapability],
) -> Option<Arc<dyn CapabilityRuntime>> {
    selector.resolve(registry, required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_resolves_nothing() {
        let reg = RuntimeRegistry::new();
        let sel = RuntimeSelector::Auto;
        assert!(resolve(&reg, &sel, &[]).is_none());
    }
}
