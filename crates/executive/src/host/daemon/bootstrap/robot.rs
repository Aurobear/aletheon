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

use crate::application::embodiment_progress::{DeferredTurnEventSink, EventEmbodimentProgress};
use crate::application::harness_factory::CognitiveSessionFactory;
use crate::composition::config::{EmbodimentProviderConfig, ResolvedRobotIntegrationConfig};

/// Build the embodiment execution port from the configured provider, together
/// with its deferred progress sink.
///
/// The port is assembled before the canonical event spine exists in the
/// bootstrap, so progress is projected through the returned
/// [`DeferredTurnEventSink`] and bound to the session event spine by
/// [`bind_robot_progress_spine`] once the spine is available (PR6).
pub async fn build_robot_embodiment_port(
    clock: Arc<dyn Clock>,
    admission: Arc<dyn fabric::AdmissionController>,
    data_dir: &std::path::Path,
    provider_config: &EmbodimentProviderConfig,
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
) -> anyhow::Result<(Arc<dyn EmbodimentExecutionPort>, Arc<DeferredTurnEventSink>)> {
    let hardware_clock: Arc<dyn hardware::MonotonicClock> =
        Arc::new(super::embodiment::HardwareClockAdapter(clock.clone()));
    let embodiment_workspace =
        fabric::WorkspacePolicy::from_resolved_roots(data_dir.to_path_buf(), Vec::new())
            .map_err(anyhow::Error::msg)
            .context("resolving embodiment workspace")?;
    let progress_sink = Arc::new(DeferredTurnEventSink::new());
    let embodiment_progress: Arc<
        dyn crate::application::embodiment_progress::EmbodimentProgressPort,
    > = Arc::new(EventEmbodimentProgress::new(progress_sink.clone()));
    let port = super::embodiment::build_embodiment_port(
        hardware_clock,
        admission,
        embodiment_progress,
        fabric::ProcessId::new(),
        fabric::PrincipalId(fabric::LOCAL_OWNER_PRINCIPAL.to_string()),
        embodiment_workspace,
        Some(provider_config.clone()),
        robot_config,
    )
    .await?;
    Ok((port, progress_sink))
}

/// Bind the deferred robot progress sink to the session event spine. Call once
/// the canonical spine exists in the bootstrap; robot turns only execute after
/// the daemon is fully assembled, so no progress event is lost before this.
pub async fn bind_robot_progress_spine(
    progress_sink: &Arc<DeferredTurnEventSink>,
    spine: Arc<dyn fabric::EventSpine>,
    session_id: impl Into<String>,
) {
    progress_sink
        .bind(Arc::new(
            crate::application::embodiment_progress::SpineTurnEventSink::new(spine, session_id),
        ))
        .await;
}

/// Compose the robot cognitive-session factory (`HarnessKind::Robot` arm).
///
/// Policy selection: a production robot harness MUST have a real policy
/// provider. A configured endpoint uses GrpcPolicyProvider (fail closed if
/// unreachable); without an endpoint the daemon fails closed rather than
/// silently degrading to a stub.
pub async fn build_robot_cognitive_session_factory(
    provider_config: &EmbodimentProviderConfig,
    robot_config: &ResolvedRobotIntegrationConfig,
    embodiment_port: Arc<dyn EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    data_dir: &std::path::Path,
    promoter: Option<Arc<dyn cognit::harness::robot::EpisodePromotionPort>>,
) -> anyhow::Result<Arc<dyn CognitiveSessionFactory>> {
    provider_config
        .validate_runtime()
        .context("validate embodiment provider configuration")?;
    let provider_device = match provider_config {
        EmbodimentProviderConfig::Simulator { device_id } => DeviceId(device_id.clone()),
        EmbodimentProviderConfig::Grpc { device_id, .. } => DeviceId(device_id.clone()),
    };
    anyhow::ensure!(
        provider_device.0 == robot_config.device_id,
        "resolved Robot device does not match embodiment provider"
    );
    let device = DeviceId(robot_config.device_id.clone());
    let policy_provider = cognit::GrpcPolicyProvider::connect(cognit::GrpcPolicyConfig {
        endpoint: robot_config.policy.endpoint.clone(),
        protocol_version: robot_config.policy.protocol_version.clone(),
        connect_timeout: robot_config.policy.connect_timeout,
        request_timeout: robot_config.policy.request_timeout,
        max_proposals: robot_config.policy.max_proposals,
    })
    .await
    .map_err(anyhow::Error::msg)
    .context("Robot Policy startup compatibility gate failed")?;
    let policy_capabilities = policy_provider.capability_snapshot();
    tracing::info!(
        provider_id = %policy_capabilities.provider_id,
        protocol_version = %policy_capabilities.protocol_version,
        server_max_proposals = policy_capabilities.server_max_proposals,
        negotiated_max_proposals = policy_capabilities.negotiated_max_proposals,
        "Robot Policy startup capability snapshot"
    );
    let policy: Arc<dyn PolicyProviderPort> = Arc::new(policy_provider);
    crate::application::robot_harness_composition::build_robot_session_factory(
        embodiment_port,
        clock,
        data_dir,
        device,
        vec![],
        policy,
        cognit::harness::robot::state::RobotHarnessConfig {
            max_retries: robot_config.max_retries,
            max_replans: robot_config.max_replans,
            default_expected_outcome: None,
            perception_max_frames: robot_config.perception.max_frames,
            required_perception: robot_config.perception.required_by_skill.clone(),
        },
        crate::application::robot_perception::RobotPerceptionRuntimeConfig {
            max_devices: robot_config.perception.max_devices,
            max_cached_frames_per_device: robot_config.perception.max_cached_frames_per_device,
            max_age: robot_config.perception.max_frame_age,
            max_total_bytes: robot_config.perception.max_total_bytes,
            allowed_uri_prefixes: robot_config.perception.allowed_uri_prefixes.clone(),
        },
        robot_config.perception.poll_interval,
        robot_config.scene_version.clone(),
        robot_config.aletheon_version.clone(),
        robot_config.bridge_protocol_digest.clone(),
        promoter,
    )
    .await
    .map_err(anyhow::Error::msg)
    .context("robot harness composition failed")
}
