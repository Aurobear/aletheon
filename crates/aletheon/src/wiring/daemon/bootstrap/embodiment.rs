//! Typed composition of the embodiment provider (simulator or gRPC gateway).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ::contracts::types::embodiment::{
    safety_capability_manifest_digest, EmbodimentExecutionPort, ExecutionEnvironment,
    SafetyCapabilityManifest,
};
use anyhow::{bail, Context};
use hardware::{
    validate_gate, Broker, DeploymentGateInput, DeviceNamespace, GrpcEmbodimentProvider,
    GrpcProviderConfig, MonotonicClock, ProviderRegistry, SimulatedEmbodiment,
};

use crate::wiring::embodiment::build_embodiment_invoker;
use crate::wiring::embodiment::EmbodimentService;
use crate::config::{
    EmbodimentProviderConfig, ResolvedRobotDeploymentGateConfig, ResolvedRobotIntegrationConfig,
};
use hardware::progress_projection::EmbodimentProgressPort;

pub struct HardwareClockAdapter(pub Arc<dyn ::contracts::Clock>);

impl MonotonicClock for HardwareClockAdapter {
    fn now(&self) -> hardware::MonotonicInstant {
        hardware::MonotonicInstant(self.0.mono_now().0)
    }
}

pub async fn build_embodiment_port(
    clock: Arc<dyn MonotonicClock>,
    admission: Arc<dyn kernel::AdmissionController>,
    progress: Arc<dyn EmbodimentProgressPort>,
    process_id: ::contracts::ProcessId,
    principal: ::contracts::PrincipalId,
    workspace: ::contracts::WorkspacePolicy,
    provider_config: Option<EmbodimentProviderConfig>,
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
) -> anyhow::Result<Arc<dyn EmbodimentExecutionPort>> {
    let config = provider_config.unwrap_or_default();
    let grpc_timeouts = config
        .checked_grpc_timeouts()
        .context("validate embodiment provider configuration")?;

    match config {
        EmbodimentProviderConfig::Simulator { device_id } => {
            let mut registry = ProviderRegistry::new();
            let device = ::contracts::types::embodiment::DeviceId(device_id.clone());
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
            ..
        } => {
            let (connect_timeout, request_timeout) = grpc_timeouts
                .context("gRPC embodiment provider is missing checked timeout values")?;
            let gate_endpoint = endpoint.clone();
            let grpc_config = GrpcProviderConfig {
                endpoint,
                protocol_version: "1.0".into(),
                connect_timeout,
                request_timeout,
                required_device_id: Some(device_id.clone()),
                allowed_protocol_digests: robot_config
                    .map(|robot| vec![robot.bridge_protocol_digest.clone()])
                    .unwrap_or_else(|| vec![hardware::grpc::BRIDGE_PROTOCOL_DIGEST.into()]),
                required_observation_schemas: robot_config
                    .map(|robot| robot.required_observation_schemas.clone())
                    .unwrap_or_default(),
                expected_execution_environment: robot_config
                    .map(|robot| robot.execution_environment)
                    .unwrap_or_default(),
                ..Default::default()
            };
            let provider = GrpcEmbodimentProvider::connect_with_clock(grpc_config, clock.clone())
                .await
                .context("failed to connect to gRPC embodiment provider")?;
            let capabilities = provider.capability_snapshot();
            let safety_manifest_digest = validate_robot_deployment_gate(
                robot_config,
                &gate_endpoint,
                &device_id,
                capabilities.execution_environment,
                &capabilities.safety_manifest,
            )?;
            tracing::info!(
                provider_id = %capabilities.provider_id,
                protocol_version = %capabilities.protocol_version,
                device_count = capabilities.device_ids.len(),
                max_message_bytes = capabilities.max_message_bytes,
                max_progress_hz = capabilities.max_progress_hz,
                protocol_digest = %capabilities.protocol_digest,
                skill_descriptor_digest = %capabilities.skill_descriptor_digest,
                observation_schema_count = capabilities.observation_schemas.len(),
                execution_environment = capabilities.execution_environment.as_str(),
                safety_manifest_digest = %safety_manifest_digest,
                device_serial = %capabilities.safety_manifest.device_serial,
                watchdog = capabilities.safety_manifest.watchdog,
                watchdog_timeout_ms = capabilities.safety_manifest.watchdog_timeout_ms,
                heartbeat = capabilities.safety_manifest.heartbeat,
                heartbeat_interval_ms = capabilities.safety_manifest.heartbeat_interval_ms,
                heartbeat_timeout_ms = capabilities.safety_manifest.heartbeat_timeout_ms,
                emergency_stop = capabilities.safety_manifest.emergency_stop,
                joint_limits = capabilities.safety_manifest.joint_limits,
                velocity_limits = capabilities.safety_manifest.velocity_limits,
                torque_or_current_limits = capabilities.safety_manifest.torque_or_current_limits,
                control_ownership = capabilities.safety_manifest.control_ownership,
                safe_stop = capabilities.safety_manifest.safe_stop,
                independent_hard_stop = capabilities.safety_manifest.independent_hard_stop,
                limits_digest = %capabilities.safety_manifest.limits_digest,
                skill_count = capabilities.skill_descriptors.len(),
                health_component_count = capabilities.health.components.len(),
                "Robot bridge startup capability snapshot"
            );

            let mut registry = ProviderRegistry::new();
            registry.register(
                ::contracts::types::embodiment::DeviceId(device_id.clone()),
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

fn validate_robot_deployment_gate(
    robot_config: Option<&ResolvedRobotIntegrationConfig>,
    endpoint: &str,
    device_id: &str,
    environment: ExecutionEnvironment,
    manifest: &SafetyCapabilityManifest,
) -> anyhow::Result<String> {
    let manifest_digest = safety_capability_manifest_digest(environment, manifest)
        .map_err(anyhow::Error::msg)
        .context("digest Bridge safety capability manifest")?;
    let Some(robot) = robot_config else {
        if environment != ExecutionEnvironment::Simulation {
            bail!("HIL/real Bridge requires a resolved Robot deployment gate");
        }
        return Ok(manifest_digest);
    };
    if robot.execution_environment != environment {
        bail!(
            "Robot deployment environment changed after capability validation: expected={} actual={}",
            robot.execution_environment.as_str(),
            environment.as_str()
        );
    }
    if environment == ExecutionEnvironment::Simulation {
        if robot.deployment_gate.is_some() {
            bail!("simulation Robot profile cannot carry a HIL/real deployment gate");
        }
        return Ok(manifest_digest);
    }

    let gate = robot.deployment_gate.as_ref().ok_or_else(|| {
        anyhow::anyhow!("HIL/real Robot profile is missing its resolved deployment gate")
    })?;
    validate_pinned_safety_facts(gate, &manifest_digest, manifest)?;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_millis()
        .try_into()
        .context("system clock milliseconds exceed i64")?;
    let namespace = match environment {
        ExecutionEnvironment::Simulation => DeviceNamespace::Simulation,
        ExecutionEnvironment::Hil => DeviceNamespace::Hil,
        ExecutionEnvironment::Real => DeviceNamespace::Production,
    };
    let result = validate_gate(&DeploymentGateInput {
        namespace,
        device_id: device_id.to_owned(),
        device_serial: gate.device_serial.clone(),
        endpoint_identity: endpoint.to_owned(),
        manifest_digest: gate.safety_manifest_digest.clone(),
        limits_digest: gate.limits_digest.clone(),
        evidence_digest: gate.evidence_digest.clone(),
        evidence_expiry_ms: gate.evidence_expiry_unix_ms,
        now_ms,
    });
    if !result.passed {
        bail!(
            "Robot deployment gate rejected startup: {}",
            result.failures.join("; ")
        );
    }
    Ok(manifest_digest)
}

fn validate_pinned_safety_facts(
    gate: &ResolvedRobotDeploymentGateConfig,
    manifest_digest: &str,
    manifest: &SafetyCapabilityManifest,
) -> anyhow::Result<()> {
    if manifest.device_serial != gate.device_serial {
        bail!(
            "Bridge device serial does not match deployment gate: expected={} actual={}",
            gate.device_serial,
            manifest.device_serial
        );
    }
    if manifest_digest != gate.safety_manifest_digest {
        bail!(
            "Bridge safety manifest digest does not match deployment gate: expected={} actual={}",
            gate.safety_manifest_digest,
            manifest_digest
        );
    }
    if manifest.limits_digest != gate.limits_digest {
        bail!(
            "Bridge limits digest does not match deployment gate: expected={} actual={}",
            gate.limits_digest,
            manifest.limits_digest
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> SafetyCapabilityManifest {
        SafetyCapabilityManifest {
            device_serial: "SN-1".into(),
            watchdog: true,
            watchdog_timeout_ms: 1_000,
            heartbeat: true,
            heartbeat_interval_ms: 100,
            heartbeat_timeout_ms: 500,
            emergency_stop: true,
            joint_limits: true,
            velocity_limits: true,
            torque_or_current_limits: true,
            control_ownership: true,
            safe_stop: true,
            independent_hard_stop: true,
            limits_digest: "b".repeat(64),
        }
    }

    #[test]
    fn deployment_gate_pins_serial_manifest_and_limits() {
        let manifest = manifest();
        let digest = safety_capability_manifest_digest(ExecutionEnvironment::Real, &manifest)
            .expect("manifest digest");
        let mut gate = ResolvedRobotDeploymentGateConfig {
            device_serial: manifest.device_serial.clone(),
            safety_manifest_digest: digest.clone(),
            limits_digest: manifest.limits_digest.clone(),
            evidence_digest: "c".repeat(64),
            evidence_expiry_unix_ms: i64::MAX,
        };
        validate_pinned_safety_facts(&gate, &digest, &manifest).expect("matching gate");

        gate.device_serial = "different".into();
        assert!(validate_pinned_safety_facts(&gate, &digest, &manifest)
            .unwrap_err()
            .to_string()
            .contains("serial"));
        gate.device_serial = manifest.device_serial.clone();
        gate.safety_manifest_digest = "d".repeat(64);
        assert!(validate_pinned_safety_facts(&gate, &digest, &manifest)
            .unwrap_err()
            .to_string()
            .contains("manifest digest"));
        gate.safety_manifest_digest = digest.clone();
        gate.limits_digest = "e".repeat(64);
        assert!(validate_pinned_safety_facts(&gate, &digest, &manifest)
            .unwrap_err()
            .to_string()
            .contains("limits digest"));
    }
}
