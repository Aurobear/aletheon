//! Typed composition of the default simulation embodiment provider.

use std::sync::Arc;

use fabric::EmbodimentExecutionPort;
use hardware::{Broker, MonotonicClock, ProviderRegistry, SimulatedEmbodiment};

use crate::service::embodiment_authority::EmbodimentAuthorityPort;
use crate::service::embodiment_progress::EmbodimentProgressPort;
use crate::service::embodiment_service::EmbodimentService;

#[allow(dead_code)] // Enabled only after P1-D registers the complete tool surface.
pub fn build_embodiment_port(
    clock: Arc<dyn MonotonicClock>,
    authority: Arc<dyn EmbodimentAuthorityPort>,
    progress: Arc<dyn EmbodimentProgressPort>,
) -> Arc<dyn EmbodimentExecutionPort> {
    let mut registry = ProviderRegistry::new();
    registry.register(
        fabric::DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock.clone())),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    Arc::new(EmbodimentService::new(broker, authority, progress))
}
