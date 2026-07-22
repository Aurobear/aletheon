//! Typed composition of the default simulation embodiment provider.

use std::sync::Arc;

use fabric::types::embodiment::EmbodimentExecutionPort;
use hardware::{Broker, MonotonicClock, ProviderRegistry, SimulatedEmbodiment};

use crate::service::embodiment_authority::build_embodiment_invoker;
use crate::service::embodiment_progress::EmbodimentProgressPort;
use crate::service::embodiment_service::EmbodimentService;

pub struct HardwareClockAdapter(pub Arc<dyn fabric::Clock>);

impl MonotonicClock for HardwareClockAdapter {
    fn now(&self) -> hardware::MonotonicInstant {
        hardware::MonotonicInstant(self.0.mono_now().0)
    }
}

pub fn build_embodiment_port(
    clock: Arc<dyn MonotonicClock>,
    admission: Arc<dyn fabric::AdmissionController>,
    progress: Arc<dyn EmbodimentProgressPort>,
    process_id: fabric::ProcessId,
    principal: fabric::PrincipalId,
    workspace: fabric::WorkspacePolicy,
) -> Arc<dyn EmbodimentExecutionPort> {
    let mut registry = ProviderRegistry::new();
    registry.register(
        fabric::types::embodiment::DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock.clone())),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let (invoker, active) = build_embodiment_invoker(admission, broker.clone(), progress);
    Arc::new(EmbodimentService::new(
        broker, invoker, active, process_id, principal, workspace,
    ))
}
