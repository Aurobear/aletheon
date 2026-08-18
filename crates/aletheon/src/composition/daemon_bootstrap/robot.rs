//! Robot embodiment bootstrap helpers.
//!
//! Kept out of `request.rs` so the request composition stage stays under its
//! line budget. Holds the robot-specific tail of the bootstrap: building the
//! embodiment execution port from the daemon config, and composing the
//! optional Robot cognitive-session factory with a production policy
//! provider (fail closed — never a silent stub).

use std::sync::Arc;

use ::contracts::types::embodiment::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, SkillDescriptor, SkillDispatchError,
    SkillRequest, SkillResult,
};
use ::contracts::Clock;
use anyhow::Context;
use cognit::ports::policy_provider::PolicyProviderPort;

use crate::config::{EmbodimentProviderConfig, ResolvedRobotIntegrationConfig};
use cognit::harness::{
    CognitiveSessionFactory, RobotSessionCapability, TargetRoutedCognitiveSessionFactory,
};
use hardware::progress_projection::{DeferredTurnEventSink, EventEmbodimentProgress};

/// Fail-closed tool backend retained when an optional configured Robot bridge
/// is unavailable during bootstrap. Keeping this port registered lets General
/// turns retain a stable tool catalog while every Robot operation returns a
/// typed provider error and the target router exposes no Robot capability.
pub struct UnavailableEmbodimentPort {
    reason: String,
}

impl UnavailableEmbodimentPort {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn unavailable(&self) -> SkillDispatchError {
        SkillDispatchError::NoProvider(self.reason.clone())
    }
}

#[async_trait::async_trait]
impl EmbodimentExecutionPort for UnavailableEmbodimentPort {
    async fn observe(
        &self,
        _device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, SkillDispatchError> {
        Err(self.unavailable())
    }

    async fn get_state(
        &self,
        _device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, SkillDispatchError> {
        Err(self.unavailable())
    }

    async fn list_skills(
        &self,
        _device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, SkillDispatchError> {
        Err(self.unavailable())
    }

    async fn execute_skill(
        &self,
        _request: SkillRequest,
    ) -> Result<SkillResult, SkillDispatchError> {
        Err(self.unavailable())
    }

    async fn cancel(
        &self,
        _operation_id: &::contracts::OperationId,
    ) -> Result<(), SkillDispatchError> {
        Err(self.unavailable())
    }

    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), SkillDispatchError> {
        Err(self.unavailable())
    }
}

/// Build the embodiment execution port from the configured provider, together
/// with its deferred progress sink.
///
/// The port is assembled before the canonical event spine exists in the
/// bootstrap, so progress is projected through the returned
/// [`DeferredTurnEventSink`] and bound to the session event spine by
/// [`bind_robot_progress_spine`] once the spine is available (PR6).
pub async fn build_robot_embodiment_port(
    clock: Arc<dyn Clock>,
    admission: Arc<dyn kernel::AdmissionController>,
    data_dir: &std::path::Path,
    provider_config: &EmbodimentProviderConfig,
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
) -> anyhow::Result<(Arc<dyn EmbodimentExecutionPort>, Arc<DeferredTurnEventSink>)> {
    let hardware_clock: Arc<dyn hardware::MonotonicClock> =
        Arc::new(super::embodiment::HardwareClockAdapter(clock.clone()));
    let embodiment_workspace =
        ::contracts::WorkspacePolicy::from_resolved_roots(data_dir.to_path_buf(), Vec::new())
            .map_err(anyhow::Error::msg)
            .context("resolving embodiment workspace")?;
    let progress_sink = Arc::new(DeferredTurnEventSink::new());
    let embodiment_progress: Arc<dyn hardware::progress_projection::EmbodimentProgressPort> =
        Arc::new(EventEmbodimentProgress::new(progress_sink.clone()));
    let port = super::embodiment::build_embodiment_port(
        hardware_clock,
        admission,
        embodiment_progress,
        ::contracts::ProcessId::new(),
        ::contracts::PrincipalId(application::LOCAL_OWNER_PRINCIPAL.to_string()),
        embodiment_workspace,
        Some(provider_config.clone()),
        robot_config,
    )
    .await?;
    Ok((port, progress_sink))
}

/// Compose the optional bridge without making General daemon availability
/// depend on Robot startup. A configured but unavailable bridge is represented
/// by a fail-closed tool port and `available = false`; an unexpected failure in
/// the default unconfigured simulator path remains a bootstrap error.
pub async fn build_resilient_robot_embodiment_port(
    clock: Arc<dyn Clock>,
    admission: Arc<dyn kernel::AdmissionController>,
    data_dir: &std::path::Path,
    provider_config: &EmbodimentProviderConfig,
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
) -> anyhow::Result<(
    Arc<dyn EmbodimentExecutionPort>,
    Arc<DeferredTurnEventSink>,
    bool,
)> {
    match build_robot_embodiment_port(clock, admission, data_dir, provider_config, robot_config)
        .await
    {
        Ok((port, progress)) => Ok((port, progress, true)),
        Err(error) if robot_config.is_some() => {
            tracing::warn!(%error, "Robot bridge unavailable; General turns remain enabled");
            Ok((
                Arc::new(UnavailableEmbodimentPort::new(
                    "configured Robot bridge is unavailable",
                )),
                Arc::new(DeferredTurnEventSink::new()),
                false,
            ))
        }
        Err(error) => Err(error),
    }
}

/// Bind the deferred robot progress sink to the session event spine. Call once
/// the canonical spine exists in the bootstrap; robot turns only execute after
/// the daemon is fully assembled, so no progress event is lost before this.
pub async fn bind_robot_progress_spine(
    progress_sink: &Arc<DeferredTurnEventSink>,
    spine: Arc<dyn runtime::EventSpine>,
    session_id: impl Into<String>,
) {
    progress_sink
        .bind(Arc::new(
            hardware::progress_projection::SpineTurnEventSink::new(spine, session_id),
        ))
        .await;
}

/// Compose the optional robot cognitive-session factory.
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
    crate::composition::robot_harness::build_robot_session_factory(
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
        cognit::harness::robot::perception_store::RobotPerceptionRuntimeConfig {
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

/// Build the one per-turn cognition router while keeping every Robot startup
/// dependency optional for General turns.
#[allow(clippy::too_many_arguments)]
pub async fn build_target_routed_cognition(
    general: Arc<dyn CognitiveSessionFactory>,
    provider_config: &EmbodimentProviderConfig,
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
    robot_transport_available: bool,
    embodiment_port: Arc<dyn EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    data_dir: &std::path::Path,
    fact_use_cases: Arc<dyn mnemosyne::FactUseCases>,
    legacy_harness_kind: cognit::harness::HarnessKind,
) -> Arc<dyn CognitiveSessionFactory> {
    let robot = if let Some(robot) = robot_config.filter(|_| robot_transport_available) {
        let promoter = Some(
            Arc::new(mnemosyne::episode_promotion::MnemosyneEpisodePromoter::new(
                fact_use_cases,
            )) as Arc<dyn cognit::harness::robot::EpisodePromotionPort>,
        );
        match build_robot_cognitive_session_factory(
            provider_config,
            robot,
            embodiment_port,
            clock,
            data_dir,
            promoter,
        )
        .await
        {
            Ok(factory) => Some(RobotSessionCapability::new(
                factory,
                DeviceId(robot.device_id.clone()),
                robot.execution_environment,
            )),
            Err(error) => {
                tracing::warn!(%error, "Robot capability unavailable; General turns remain enabled");
                None
            }
        }
    } else {
        None
    };
    if legacy_harness_kind == cognit::harness::HarnessKind::Robot {
        tracing::warn!(
            "agent.harness_kind=robot is deprecated as a per-prompt default; it now enables only optional Robot capability and all turns default to General"
        );
    }
    Arc::new(TargetRoutedCognitiveSessionFactory::new(general, robot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unavailable_port_fails_closed_for_reads_and_actuation() {
        let port = UnavailableEmbodimentPort::new("bridge offline");
        let device = DeviceId("robot-1".into());
        assert!(matches!(
            port.observe(&device).await,
            Err(SkillDispatchError::NoProvider(reason)) if reason == "bridge offline"
        ));
        assert!(matches!(
            port.execute_skill(SkillRequest {
                skill: ::contracts::types::embodiment::SkillId("move".into()),
                device,
                parameters: serde_json::json!({}),
            })
            .await,
            Err(SkillDispatchError::NoProvider(reason)) if reason == "bridge offline"
        ));
    }
}
