//! Typed composition of the embodiment provider (simulator or gRPC gateway).

use std::sync::Arc;

use anyhow::Context;
use fabric::types::embodiment::EmbodimentExecutionPort;
use hardware::{
    Broker, GrpcEmbodimentProvider, GrpcProviderConfig, MonotonicClock, ProviderRegistry,
    SimulatedEmbodiment,
};

use crate::application::embodiment_authority::build_embodiment_invoker;
use crate::application::embodiment_progress::EmbodimentProgressPort;
use crate::application::embodiment_service::EmbodimentService;
use crate::composition::config::EmbodimentProviderConfig;

pub struct HardwareClockAdapter(pub Arc<dyn fabric::Clock>);

impl MonotonicClock for HardwareClockAdapter {
    fn now(&self) -> hardware::MonotonicInstant {
        hardware::MonotonicInstant(self.0.mono_now().0)
    }
}

pub async fn build_embodiment_port(
    clock: Arc<dyn MonotonicClock>,
    admission: Arc<dyn fabric::AdmissionController>,
    progress: Arc<dyn EmbodimentProgressPort>,
    process_id: fabric::ProcessId,
    principal: fabric::PrincipalId,
    workspace: fabric::WorkspacePolicy,
    provider_config: Option<EmbodimentProviderConfig>,
) -> anyhow::Result<Arc<dyn EmbodimentExecutionPort>> {
    let config = provider_config.unwrap_or_default();

    match config {
        EmbodimentProviderConfig::Simulator { device_id } => {
            let mut registry = ProviderRegistry::new();
            let device = fabric::types::embodiment::DeviceId(device_id.clone());
            registry.register(
                device.clone(),
                Arc::new(SimulatedEmbodiment::mobile_robot(&device_id, clock.clone())),
            );
            let broker = Arc::new(Broker::new(Arc::new(registry), clock));
            let (invoker, active) = build_embodiment_invoker(admission, broker.clone(), progress);
            Ok(Arc::new(EmbodimentService::new(
                broker, invoker, active, process_id, principal, workspace,
            )))
        }
        EmbodimentProviderConfig::Grpc {
            device_id,
            endpoint,
            connect_timeout_ms,
            request_timeout_ms,
        } => {
            let grpc_config = GrpcProviderConfig {
                endpoint,
                protocol_version: "1.0".into(),
                connect_timeout: std::time::Duration::from_millis(connect_timeout_ms),
                request_timeout: std::time::Duration::from_millis(request_timeout_ms),
                ..Default::default()
            };
            let provider = GrpcEmbodimentProvider::connect(grpc_config)
                .await
                .context("failed to connect to gRPC embodiment provider")?;

            let mut registry = ProviderRegistry::new();
            registry.register(
                fabric::types::embodiment::DeviceId(device_id.clone()),
                Arc::new(provider),
            );
            let broker = Arc::new(Broker::new(Arc::new(registry), clock));
            let (invoker, active) = build_embodiment_invoker(admission, broker.clone(), progress);
            Ok(Arc::new(EmbodimentService::new(
                broker, invoker, active, process_id, principal, workspace,
            )))
        }
    }
}
