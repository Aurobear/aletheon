//! Robot embodiment bootstrap helpers.
//!
//! Kept out of `request.rs` so the request composition stage stays under its
//! line budget. Holds the robot-specific tail of the bootstrap: building the
//! embodiment execution port from the daemon config, and composing the
//! `HarnessKind::Robot` cognitive-session factory with a production policy
//! provider (fail closed — never a silent stub).

use std::sync::Arc;

use anyhow::Context;
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{DeviceId, EmbodimentExecutionPort};
use fabric::Clock;

use crate::application::harness_factory::CognitiveSessionFactory;
use crate::composition::config::EmbodimentProviderConfig;

/// Build the embodiment execution port from the configured provider.
///
/// Canonical embodiment progress → turn event projection surfaces into daemon
/// logs today; the session/UI event projection replaces the tracing sink in the
/// robot composition tail.
pub async fn build_robot_embodiment_port(
    clock: Arc<dyn Clock>,
    admission: Arc<dyn fabric::AdmissionController>,
    data_dir: &std::path::Path,
    provider_config: &EmbodimentProviderConfig,
) -> anyhow::Result<Arc<dyn EmbodimentExecutionPort>> {
    let hardware_clock: Arc<dyn hardware::MonotonicClock> =
        Arc::new(super::embodiment::HardwareClockAdapter(clock.clone()));
    let embodiment_workspace =
        fabric::WorkspacePolicy::from_resolved_roots(data_dir.to_path_buf(), Vec::new())
            .map_err(anyhow::Error::msg)
            .context("resolving embodiment workspace")?;
    let embodiment_progress: Arc<
        dyn crate::application::embodiment_progress::EmbodimentProgressPort,
    > = Arc::new(crate::application::embodiment_progress::EventEmbodimentProgress::new(
        Arc::new(crate::application::embodiment_progress::TracingTurnEventSink),
    ));
    super::embodiment::build_embodiment_port(
        hardware_clock,
        admission,
        embodiment_progress,
        fabric::ProcessId::new(),
        fabric::PrincipalId(fabric::LOCAL_OWNER_PRINCIPAL.to_string()),
        embodiment_workspace,
        Some(provider_config.clone()),
    )
    .await
}

/// Compose the robot cognitive-session factory (`HarnessKind::Robot` arm).
///
/// Policy selection: a production robot harness MUST have a real policy
/// provider. A configured endpoint uses GrpcPolicyProvider (fail closed if
/// unreachable); without an endpoint the daemon fails closed rather than
/// silently degrading to a stub.
pub async fn build_robot_cognitive_session_factory(
    provider_config: &EmbodimentProviderConfig,
    embodiment_port: Arc<dyn EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    data_dir: &std::path::Path,
) -> anyhow::Result<Arc<dyn CognitiveSessionFactory>> {
    let device = match provider_config {
        EmbodimentProviderConfig::Simulator { device_id } => DeviceId(device_id.clone()),
        EmbodimentProviderConfig::Grpc { device_id, .. } => DeviceId(device_id.clone()),
    };
    let policy: Arc<dyn PolicyProviderPort> = match std::env::var("ALETHEON_POLICY_ENDPOINT") {
        Ok(endpoint) => Arc::new(
            cognit::GrpcPolicyProvider::connect(cognit::GrpcPolicyConfig {
                endpoint,
                ..Default::default()
            })
            .await
            .map_err(anyhow::Error::msg)
            .context("robot policy endpoint configured but unreachable")?,
        ),
        Err(_) => anyhow::bail!(
            "HarnessKind::Robot requires ALETHEON_POLICY_ENDPOINT for a production policy provider"
        ),
    };
    crate::application::robot_harness_composition::build_robot_session_factory(
        embodiment_port,
        clock,
        data_dir,
        device,
        vec![],
        policy,
        "",
        option_env!("CARGO_PKG_VERSION").unwrap_or("unknown"),
        "",
    )
    .await
    .map_err(anyhow::Error::msg)
    .context("robot harness composition failed")
}
