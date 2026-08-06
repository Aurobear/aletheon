//! gRPC embodiment provider over the vendor-neutral gateway contract.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fabric::types::embodiment::{
    skill_descriptor_digest, DeviceId, EmbodiedObservation, ExecutionEnvironment,
    SafetyCapabilityManifest, SkillDescriptor, SkillResult,
};
use tonic::transport::Channel;

use crate::grpc::convert;
use crate::grpc::error::map_error;
use crate::grpc::wire::embodiment_gateway_client::EmbodimentGatewayClient;
use crate::grpc::wire::{self, RequestMeta};
use crate::skill::SkillProgressSink;
use crate::{
    CancelAck, EmbodimentProvider, MonotonicClock, MonotonicInstant, ProviderError, StopReceipt,
    ValidatedSkillCommand,
};

/// Configuration for a gRPC embodiment provider.
#[derive(Debug, Clone)]
pub struct GrpcProviderConfig {
    /// gRPC endpoint URL (e.g. "http://127.0.0.1:50051").
    pub endpoint: String,
    /// Protocol version sent in every request header.
    pub protocol_version: String,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Per-RPC request timeout.
    pub request_timeout: Duration,
    /// Max gRPC message size in bytes.
    pub max_decoding_message_size: usize,
    /// Device that must be advertised by the startup capability snapshot.
    /// Connection fails closed when this is absent.
    pub required_device_id: Option<String>,
    /// Reviewed protocol digests accepted by this runtime generation.
    pub allowed_protocol_digests: Vec<String>,
    /// Observation schemas required before RobotHarness can be exposed.
    pub required_observation_schemas: Vec<ObservationSchemaRequirement>,
    /// Operator-selected deployment gate. The provider-attested environment must
    /// match exactly; configuration cannot promote a simulation bridge to real.
    pub expected_execution_environment: ExecutionEnvironment,
}

impl Default for GrpcProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50051".into(),
            protocol_version: "1.0".into(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            max_decoding_message_size: 16 * 1024 * 1024,
            required_device_id: None,
            allowed_protocol_digests: vec![crate::grpc::BRIDGE_PROTOCOL_DIGEST.into()],
            required_observation_schemas: Vec::new(),
            expected_execution_environment: ExecutionEnvironment::Simulation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservationSchemaRequirement {
    pub schema: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHealthComponentSnapshot {
    pub component: String,
    pub state: String,
    pub detail: String,
    pub observed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHealthSnapshot {
    pub state: String,
    pub components: Vec<BridgeHealthComponentSnapshot>,
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeStartupError {
    #[error("invalid Bridge endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("Bridge connection failed: {0}")]
    Connection(String),
    #[error("Bridge Health RPC failed: {0}")]
    HealthRpc(String),
    #[error("Bridge is not ready: {state}")]
    Unhealthy { state: String },
    #[error("invalid Bridge health snapshot: {0}")]
    InvalidHealth(String),
    #[error("Bridge GetCapabilities RPC failed: {0}")]
    CapabilitiesRpc(String),
    #[error("invalid Bridge capability snapshot: {0}")]
    InvalidCapabilities(String),
    #[error("Bridge protocol mismatch: client={client}, server={server}")]
    ProtocolMismatch { client: String, server: String },
    #[error("Bridge protocol digest is not allowed: {actual}")]
    ProtocolDigestMismatch { actual: String },
    #[error("Bridge ListSkills RPC failed: {0}")]
    SkillsRpc(String),
    #[error("Bridge exposed no skills for device {device}")]
    NoSkills { device: String },
    #[error("invalid Bridge skill descriptor: {0}")]
    InvalidSkillDescriptor(String),
    #[error("Bridge is missing required observation schema {schema}/v{schema_version}")]
    MissingObservationSchema { schema: String, schema_version: u32 },
    #[error("Bridge execution environment mismatch: expected={expected} actual={actual}")]
    ExecutionEnvironmentMismatch { expected: String, actual: String },
    #[error("Bridge is missing mandatory safety capabilities for {environment}: {missing}")]
    MissingSafetyCapabilities {
        environment: String,
        missing: String,
    },
    #[error("Bridge Heartbeat RPC failed: {0}")]
    HeartbeatRpc(String),
}

/// Bridge facts returned by GetCapabilities and validated before the provider
/// is exposed to Broker/Executive.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCapabilitySnapshot {
    pub protocol_version: String,
    pub protocol_digest: String,
    pub provider_id: String,
    pub device_ids: Vec<DeviceId>,
    pub max_message_bytes: u32,
    pub max_progress_hz: u32,
    pub observation_schemas: Vec<ObservationSchemaRequirement>,
    pub execution_environment: ExecutionEnvironment,
    pub safety_manifest: SafetyCapabilityManifest,
    pub skill_descriptors: Vec<SkillDescriptor>,
    pub skill_descriptor_digest: String,
    pub health: BridgeHealthSnapshot,
}

/// gRPC client provider implementing the EmbodimentProvider trait.
///
/// Connects on construction and performs a capabilities handshake.
/// All RPCs carry protocol version and deadline metadata.
pub struct GrpcEmbodimentProvider {
    client: EmbodimentGatewayClient<Channel>,
    config: GrpcProviderConfig,
    clock: Option<Arc<dyn MonotonicClock>>,
    fallback_mono_base: Instant,
    capabilities: BridgeCapabilitySnapshot,
    control_session_id: String,
    heartbeat_healthy: Arc<AtomicBool>,
    heartbeat_task: tokio::task::AbortHandle,
}

impl GrpcEmbodimentProvider {
    /// Connect to the bridge and perform a capabilities handshake.
    pub async fn connect(config: GrpcProviderConfig) -> Result<Self, BridgeStartupError> {
        Self::connect_inner(config, None).await
    }

    /// Connect with the monotonic clock that issues permits and leases.
    ///
    /// A shared clock is required for skill execution because domain deadlines
    /// are process-local monotonic instants while the wire protocol carries
    /// Unix timestamps.
    pub async fn connect_with_clock(
        config: GrpcProviderConfig,
        clock: Arc<dyn MonotonicClock>,
    ) -> Result<Self, BridgeStartupError> {
        Self::connect_inner(config, Some(clock)).await
    }

    async fn connect_inner(
        config: GrpcProviderConfig,
        clock: Option<Arc<dyn MonotonicClock>>,
    ) -> Result<Self, BridgeStartupError> {
        let required_device = config
            .required_device_id
            .as_deref()
            .map(str::trim)
            .filter(|device| !device.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                BridgeStartupError::InvalidCapabilities(
                    "required_device_id is mandatory for a production provider".into(),
                )
            })?;
        let endpoint = tonic::transport::Endpoint::from_shared(config.endpoint.clone())
            .map_err(|error| BridgeStartupError::InvalidEndpoint(error.to_string()))?
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout);

        let channel = endpoint
            .connect()
            .await
            .map_err(|error| BridgeStartupError::Connection(error.to_string()))?;

        let mut client = EmbodimentGatewayClient::new(channel)
            .max_decoding_message_size(config.max_decoding_message_size);

        // Read-only startup gate. These calls cannot carry actuation and all run
        // before the provider is registered with Broker.
        let health = client
            .health(wire::HealthRequest {
                meta: Some(startup_meta(&config)),
            })
            .await
            .map_err(|error| BridgeStartupError::HealthRpc(error.to_string()))?
            .into_inner();
        let health = validate_health(health)?;

        let caps = client
            .get_capabilities(wire::GetCapabilitiesRequest {
                meta: Some(startup_meta(&config)),
            })
            .await
            .map_err(|error| BridgeStartupError::CapabilitiesRpc(error.to_string()))?
            .into_inner();
        let facts = validate_capability_facts(&config, caps)?;

        let skills = client
            .list_skills(wire::ListSkillsRequest {
                meta: Some(startup_meta(&config)),
                device_id: required_device.clone(),
            })
            .await
            .map_err(|error| BridgeStartupError::SkillsRpc(error.to_string()))?
            .into_inner();
        if let Some(error) = skills.error {
            return Err(BridgeStartupError::SkillsRpc(format!(
                "{}: {}",
                error.category, error.message
            )));
        }
        let mut descriptors = skills
            .skills
            .iter()
            .map(convert::to_skill_descriptor)
            .collect::<Result<Vec<_>, _>>()
            .map_err(BridgeStartupError::InvalidSkillDescriptor)?;
        if descriptors.is_empty() {
            return Err(BridgeStartupError::NoSkills {
                device: required_device,
            });
        }
        descriptors.sort_by(|left, right| left.skill.0.cmp(&right.skill.0));
        let expected_device = DeviceId(required_device.clone());
        let mut seen = std::collections::BTreeSet::new();
        for descriptor in &descriptors {
            descriptor
                .validate_contract(&expected_device)
                .map_err(BridgeStartupError::InvalidSkillDescriptor)?;
            if !seen.insert(descriptor.skill.0.clone()) {
                return Err(BridgeStartupError::InvalidSkillDescriptor(format!(
                    "duplicate skill id {}",
                    descriptor.skill.0
                )));
            }
        }
        let descriptor_digest = skill_descriptor_digest(&descriptors)
            .map_err(BridgeStartupError::InvalidSkillDescriptor)?;

        // Establish liveness before registering the provider. Thereafter a
        // provider-owned task keeps the Bridge-local watchdog fed; if the daemon
        // exits, the task exits with it and the Bridge trips independently.
        let control_session_id = fabric::OperationId::new().0.to_string();
        heartbeat_once(
            &mut client,
            &config,
            &required_device,
            &control_session_id,
            facts.safety_manifest.heartbeat_timeout_ms,
        )
        .await?;
        let heartbeat_healthy = Arc::new(AtomicBool::new(true));
        let heartbeat_task = spawn_heartbeat(
            client.clone(),
            config.clone(),
            required_device.clone(),
            control_session_id.clone(),
            facts.safety_manifest.heartbeat_interval_ms,
            facts.safety_manifest.heartbeat_timeout_ms,
            heartbeat_healthy.clone(),
        );

        let capabilities = BridgeCapabilitySnapshot {
            protocol_version: facts.protocol_version,
            protocol_digest: facts.protocol_digest,
            provider_id: facts.provider_id,
            device_ids: facts.device_ids,
            max_message_bytes: facts.max_message_bytes,
            max_progress_hz: facts.max_progress_hz,
            observation_schemas: facts.observation_schemas,
            execution_environment: facts.execution_environment,
            safety_manifest: facts.safety_manifest,
            skill_descriptors: descriptors,
            skill_descriptor_digest: descriptor_digest,
            health,
        };

        Ok(Self {
            client,
            config,
            clock,
            fallback_mono_base: Instant::now(),
            capabilities,
            control_session_id,
            heartbeat_healthy,
            heartbeat_task,
        })
    }

    pub fn capability_snapshot(&self) -> &BridgeCapabilitySnapshot {
        &self.capabilities
    }

    fn build_meta(&self) -> RequestMeta {
        RequestMeta {
            protocol_version: self.config.protocol_version.clone(),
            ..Default::default()
        }
    }

    fn build_deadline_meta(&self, deadline_ms: u64) -> RequestMeta {
        RequestMeta {
            protocol_version: self.config.protocol_version.clone(),
            deadline_unix_ms: deadline_ms as i64,
            ..Default::default()
        }
    }

    fn deadline_unix_ms(&self, expires_at: MonotonicInstant) -> Result<u64, ProviderError> {
        let clock = self.clock.as_ref().ok_or_else(|| {
            ProviderError::Rejected("gRPC skill execution requires a shared monotonic clock".into())
        })?;
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderError::Rejected("system clock precedes Unix epoch".into()))?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        Ok(monotonic_deadline_to_unix_ms(
            expires_at.0,
            clock.now().0,
            now_unix_ms,
        ))
    }
}

fn startup_meta(config: &GrpcProviderConfig) -> RequestMeta {
    RequestMeta {
        protocol_version: config.protocol_version.clone(),
        ..Default::default()
    }
}

fn validate_health(
    health: wire::HealthResponse,
) -> Result<BridgeHealthSnapshot, BridgeStartupError> {
    if let Some(error) = health.error {
        return Err(BridgeStartupError::InvalidHealth(format!(
            "{}: {}",
            error.category, error.message
        )));
    }
    let state = wire::HealthState::try_from(health.state)
        .map_err(|_| BridgeStartupError::InvalidHealth("unknown overall state".into()))?;
    if state != wire::HealthState::Ready {
        return Err(BridgeStartupError::Unhealthy {
            state: state.as_str_name().to_owned(),
        });
    }
    if health.components.is_empty() {
        return Err(BridgeStartupError::InvalidHealth(
            "component health facts are empty".into(),
        ));
    }
    let mut components = Vec::with_capacity(health.components.len());
    for component in health.components {
        if component.component.trim().is_empty() || component.observed_unix_ms <= 0 {
            return Err(BridgeStartupError::InvalidHealth(
                "component identity/time is incomplete".into(),
            ));
        }
        let component_state = wire::HealthState::try_from(component.state)
            .map_err(|_| BridgeStartupError::InvalidHealth("unknown component state".into()))?;
        if component_state == wire::HealthState::Unspecified {
            return Err(BridgeStartupError::InvalidHealth(format!(
                "component {} has an unspecified state",
                component.component
            )));
        }
        components.push(BridgeHealthComponentSnapshot {
            component: component.component,
            state: component_state.as_str_name().to_owned(),
            detail: component.detail,
            observed_unix_ms: component.observed_unix_ms,
        });
    }
    Ok(BridgeHealthSnapshot {
        state: state.as_str_name().to_owned(),
        components,
    })
}

struct ValidatedCapabilityFacts {
    protocol_version: String,
    protocol_digest: String,
    provider_id: String,
    device_ids: Vec<DeviceId>,
    max_message_bytes: u32,
    max_progress_hz: u32,
    observation_schemas: Vec<ObservationSchemaRequirement>,
    execution_environment: ExecutionEnvironment,
    safety_manifest: SafetyCapabilityManifest,
}

fn validate_capability_facts(
    config: &GrpcProviderConfig,
    caps: wire::GetCapabilitiesResponse,
) -> Result<ValidatedCapabilityFacts, BridgeStartupError> {
    if caps.protocol_version != config.protocol_version {
        return Err(BridgeStartupError::ProtocolMismatch {
            client: config.protocol_version.clone(),
            server: caps.protocol_version,
        });
    }
    if config.allowed_protocol_digests.is_empty() {
        return Err(BridgeStartupError::InvalidCapabilities(
            "allowed_protocol_digests is empty".into(),
        ));
    }
    if !config
        .allowed_protocol_digests
        .iter()
        .any(|allowed| allowed == &caps.protocol_digest)
    {
        return Err(BridgeStartupError::ProtocolDigestMismatch {
            actual: caps.protocol_digest,
        });
    }
    if caps.provider_id.trim().is_empty() {
        return Err(BridgeStartupError::InvalidCapabilities(
            "capability provider_id is empty".into(),
        ));
    }
    let device_ids = caps
        .device_ids
        .into_iter()
        .map(|device| DeviceId(device.trim().to_owned()))
        .collect::<Vec<_>>();
    if device_ids.is_empty() || device_ids.iter().any(|device| device.0.is_empty()) {
        return Err(BridgeStartupError::InvalidCapabilities(
            "capability device_ids must be non-empty".into(),
        ));
    }
    if let Some(required) = config.required_device_id.as_deref() {
        if !device_ids.iter().any(|device| device.0 == required) {
            return Err(BridgeStartupError::InvalidCapabilities(format!(
                "configured device '{required}' is absent from bridge capabilities"
            )));
        }
    }
    if caps.max_message_bytes == 0 {
        return Err(BridgeStartupError::InvalidCapabilities(
            "capability max_message_bytes must be nonzero".into(),
        ));
    }
    if caps.max_progress_hz == 0 {
        return Err(BridgeStartupError::InvalidCapabilities(
            "capability max_progress_hz must be nonzero".into(),
        ));
    }
    let mut observation_schemas = caps
        .observation_schemas
        .into_iter()
        .map(|descriptor| ObservationSchemaRequirement {
            schema: descriptor.schema.trim().to_owned(),
            schema_version: descriptor.schema_version,
        })
        .collect::<Vec<_>>();
    if observation_schemas.is_empty()
        || observation_schemas
            .iter()
            .any(|descriptor| descriptor.schema.is_empty() || descriptor.schema_version == 0)
    {
        return Err(BridgeStartupError::InvalidCapabilities(
            "observation schema facts are empty or incomplete".into(),
        ));
    }
    observation_schemas.sort();
    observation_schemas.dedup();
    for required in &config.required_observation_schemas {
        if !observation_schemas.contains(required) {
            return Err(BridgeStartupError::MissingObservationSchema {
                schema: required.schema.clone(),
                schema_version: required.schema_version,
            });
        }
    }
    let execution_environment =
        match wire::ExecutionEnvironment::try_from(caps.execution_environment).map_err(|_| {
            BridgeStartupError::InvalidCapabilities(
                "capability execution_environment is unknown".into(),
            )
        })? {
            wire::ExecutionEnvironment::Simulation => ExecutionEnvironment::Simulation,
            wire::ExecutionEnvironment::Hil => ExecutionEnvironment::Hil,
            wire::ExecutionEnvironment::Real => ExecutionEnvironment::Real,
            wire::ExecutionEnvironment::Unspecified => {
                return Err(BridgeStartupError::InvalidCapabilities(
                    "capability execution_environment is unspecified".into(),
                ))
            }
        };
    if execution_environment != config.expected_execution_environment {
        return Err(BridgeStartupError::ExecutionEnvironmentMismatch {
            expected: config.expected_execution_environment.as_str().into(),
            actual: execution_environment.as_str().into(),
        });
    }
    let safety = caps.safety_manifest.ok_or_else(|| {
        BridgeStartupError::InvalidCapabilities("capability safety_manifest is absent".into())
    })?;
    let safety_manifest = SafetyCapabilityManifest {
        device_serial: safety.device_serial.trim().to_owned(),
        watchdog: safety.watchdog,
        watchdog_timeout_ms: safety.watchdog_timeout_ms,
        heartbeat: safety.heartbeat,
        heartbeat_interval_ms: safety.heartbeat_interval_ms,
        emergency_stop: safety.emergency_stop,
        joint_limits: safety.joint_limits,
        velocity_limits: safety.velocity_limits,
        torque_or_current_limits: safety.torque_or_current_limits,
        control_ownership: safety.control_ownership,
        safe_stop: safety.safe_stop,
        independent_hard_stop: safety.independent_hard_stop,
        limits_digest: safety.limits_digest.trim().to_owned(),
        heartbeat_timeout_ms: safety.heartbeat_timeout_ms,
    };
    if safety_manifest.heartbeat_timeout_ms > 30_000 || safety_manifest.watchdog_timeout_ms > 30_000
    {
        return Err(BridgeStartupError::InvalidCapabilities(
            "capability safety timeout exceeds 30000".into(),
        ));
    }
    let missing = safety_manifest.missing_for(execution_environment);
    if !missing.is_empty() {
        return Err(BridgeStartupError::MissingSafetyCapabilities {
            environment: execution_environment.as_str().into(),
            missing: missing.join(","),
        });
    }
    Ok(ValidatedCapabilityFacts {
        protocol_version: caps.protocol_version,
        protocol_digest: caps.protocol_digest,
        provider_id: caps.provider_id.trim().to_owned(),
        device_ids,
        max_message_bytes: caps.max_message_bytes,
        max_progress_hz: caps.max_progress_hz,
        observation_schemas,
        execution_environment,
        safety_manifest,
    })
}

async fn heartbeat_once(
    client: &mut EmbodimentGatewayClient<Channel>,
    config: &GrpcProviderConfig,
    device_id: &str,
    control_session_id: &str,
    timeout_ms: u64,
) -> Result<(), BridgeStartupError> {
    let response = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        client.heartbeat(wire::HeartbeatRequest {
            meta: Some(startup_meta(config)),
            device_id: device_id.to_owned(),
            control_session_id: control_session_id.to_owned(),
        }),
    )
    .await
    .map_err(|_| BridgeStartupError::HeartbeatRpc("heartbeat timed out".into()))?
    .map_err(|error| BridgeStartupError::HeartbeatRpc(error.to_string()))?
    .into_inner();
    if let Some(error) = response.error {
        return Err(BridgeStartupError::HeartbeatRpc(format!(
            "{}: {}",
            error.category, error.message
        )));
    }
    if !response.acknowledged || response.observed_unix_ms <= 0 {
        return Err(BridgeStartupError::HeartbeatRpc(
            "Bridge did not acknowledge heartbeat with a valid observation time".into(),
        ));
    }
    Ok(())
}

fn spawn_heartbeat(
    mut client: EmbodimentGatewayClient<Channel>,
    config: GrpcProviderConfig,
    device_id: String,
    control_session_id: String,
    interval_ms: u64,
    timeout_ms: u64,
    healthy: Arc<AtomicBool>,
) -> tokio::task::AbortHandle {
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
            let response = tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                client.heartbeat(wire::HeartbeatRequest {
                    meta: Some(startup_meta(&config)),
                    device_id: device_id.clone(),
                    control_session_id: control_session_id.clone(),
                }),
            )
            .await;
            let acknowledged = match response {
                Ok(Ok(response)) => {
                    let response = response.into_inner();
                    response.acknowledged
                        && response.observed_unix_ms > 0
                        && response.error.is_none()
                }
                Ok(Err(_)) | Err(_) => false,
            };
            healthy.store(acknowledged, Ordering::Release);
        }
    });
    task.abort_handle()
}

impl Drop for GrpcEmbodimentProvider {
    fn drop(&mut self) {
        self.heartbeat_task.abort();
    }
}

fn monotonic_deadline_to_unix_ms(expires_at: u64, now_mono: u64, now_unix: u64) -> u64 {
    now_unix.saturating_add(expires_at.saturating_sub(now_mono))
}

#[async_trait]
impl EmbodimentProvider for GrpcEmbodimentProvider {
    async fn observe(&self, device: &DeviceId) -> Result<Vec<EmbodiedObservation>, ProviderError> {
        let resp = self
            .client
            .clone()
            .snapshot(wire::SnapshotRequest {
                meta: Some(self.build_meta()),
                device_id: device.0.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        let local_mono_now = self.clock.as_ref().map_or_else(
            || fabric::MonoTime(self.fallback_mono_base.elapsed().as_millis() as u64),
            |clock| fabric::MonoTime(clock.now().0),
        );
        let local_unix_now_ms = current_unix_ms().map_err(ProviderError::Rejected)?;
        resp.observations
            .iter()
            .map(|o| {
                convert::to_observation(o, local_mono_now, local_unix_now_ms)
                    .map_err(ProviderError::Rejected)
            })
            .collect()
    }

    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, ProviderError> {
        let mut observations = self.observe(device).await?;
        Ok(observations.pop())
    }

    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError> {
        if self.config.required_device_id.as_deref() != Some(device.0.as_str()) {
            return Err(ProviderError::Rejected(format!(
                "device {} is outside the startup capability snapshot",
                device.0
            )));
        }
        Ok(self.capabilities.skill_descriptors.clone())
    }

    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError> {
        if !self.heartbeat_healthy.load(Ordering::Acquire) {
            return Err(ProviderError::Disconnected);
        }
        let request = command.request();
        let permit = command.permit();
        let lease = command.lease();

        let params = crate::grpc::convert::json_to_struct(&request.parameters);

        let permit_expires_unix_ms = self.deadline_unix_ms(permit.expires_at)?;
        let lease_expires_unix_ms = self.deadline_unix_ms(lease.expires_at)?;
        let wire_request = wire::ExecuteSkillRequest {
            meta: Some(self.build_deadline_meta(permit_expires_unix_ms)),
            operation_id: permit.operation.0.clone(),
            device_id: request.device.0.clone(),
            skill_id: request.skill.0.clone(),
            parameters: Some(params),
            lease_expires_unix_ms: lease_expires_unix_ms as i64,
            control_session_id: self.control_session_id.clone(),
        };

        let mut stream = self
            .client
            .clone()
            .execute_skill(wire_request)
            .await
            .map_err(map_status)?
            .into_inner();

        let mut final_result: Option<SkillResult> = None;

        while let Some(event) = stream
            .message()
            .await
            .map_err(|_s| ProviderError::Disconnected)?
        {
            match event.event {
                Some(wire::execute_skill_event::Event::Accepted(accepted)) => {
                    // Accepted — no domain equivalent to forward, just log
                    let _ = accepted.accepted_unix_ms;
                }
                Some(wire::execute_skill_event::Event::Progress(wp)) => {
                    if let Ok(dp) = convert::to_skill_progress(&wp) {
                        progress.progress(dp).await;
                    }
                }
                Some(wire::execute_skill_event::Event::Result(wr)) => {
                    let dr = convert::to_skill_result(&wr).map_err(ProviderError::Rejected)?;
                    // Validate identity
                    if dr.device != request.device {
                        return Err(ProviderError::Rejected("result device mismatch".into()));
                    }
                    if dr.skill != request.skill {
                        return Err(ProviderError::Rejected("result skill mismatch".into()));
                    }
                    final_result = Some(dr);
                    break; // terminal
                }
                Some(wire::execute_skill_event::Event::Error(error)) => {
                    return Err(map_error(&error));
                }
                None => {
                    // Empty event — continue
                }
            }
        }

        final_result.ok_or(ProviderError::Rejected(
            "stream ended without terminal result".into(),
        ))
    }

    async fn cancel(
        &self,
        device: &DeviceId,
        operation: &crate::DeviceOperationId,
    ) -> Result<CancelAck, ProviderError> {
        let resp = self
            .client
            .clone()
            .cancel(wire::CancelRequest {
                meta: Some(self.build_meta()),
                operation_id: operation.0.clone(),
                device_id: device.0.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        Ok(CancelAck {
            device: device.clone(),
        })
    }

    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError> {
        let resp = self
            .client
            .clone()
            .safe_stop(wire::SafeStopRequest {
                meta: Some(self.build_meta()),
                device_id: device.0.clone(),
                reason: "requested".into(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        Ok(StopReceipt {
            device: device.clone(),
        })
    }
}

/// Map a tonic::Status to a ProviderError.
fn map_status(status: tonic::Status) -> ProviderError {
    match status.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => ProviderError::Timeout,
        tonic::Code::Unimplemented => ProviderError::Rejected(status.message().into()),
        _ => ProviderError::Rejected(format!(
            "gRPC {}: {}",
            status.code().description(),
            status.message()
        )),
    }
}

fn current_unix_ms() -> Result<i64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes Unix epoch".to_string())?
        .as_millis();
    i64::try_from(millis).map_err(|_| "system clock does not fit Unix milliseconds".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> wire::GetCapabilitiesResponse {
        wire::GetCapabilitiesResponse {
            protocol_version: "1.0".into(),
            provider_id: "kuavo-bridge".into(),
            device_ids: vec!["kuavo-mujoco-01".into()],
            max_message_bytes: 1024,
            max_progress_hz: 20,
            protocol_digest: crate::grpc::BRIDGE_PROTOCOL_DIGEST.into(),
            observation_schemas: vec![wire::ObservationSchemaDescriptor {
                schema: "base_pose".into(),
                schema_version: 1,
            }],
            execution_environment: wire::ExecutionEnvironment::Simulation.into(),
            safety_manifest: Some(wire::SafetyCapabilityManifest {
                device_serial: String::new(),
                watchdog: true,
                watchdog_timeout_ms: 1_000,
                heartbeat: true,
                heartbeat_interval_ms: 100,
                emergency_stop: false,
                joint_limits: false,
                velocity_limits: true,
                torque_or_current_limits: false,
                control_ownership: true,
                safe_stop: true,
                independent_hard_stop: false,
                limits_digest: "limits-sim".into(),
                heartbeat_timeout_ms: 1_000,
            }),
        }
    }

    fn health() -> wire::HealthResponse {
        wire::HealthResponse {
            state: wire::HealthState::Ready.into(),
            components: vec![wire::ComponentHealth {
                component: "ros_master".into(),
                state: wire::HealthState::Ready.into(),
                detail: "ready".into(),
                observed_unix_ms: 1_700_000_000_000,
            }],
            error: None,
        }
    }

    #[test]
    fn converts_remaining_monotonic_duration_to_unix_deadline() {
        assert_eq!(
            monotonic_deadline_to_unix_ms(12_500, 10_000, 1_700_000_000_000),
            1_700_000_002_500
        );
    }

    #[test]
    fn expired_monotonic_deadline_maps_to_current_unix_time() {
        assert_eq!(
            monotonic_deadline_to_unix_ms(9_000, 10_000, 1_700_000_000_000),
            1_700_000_000_000
        );
    }

    #[test]
    fn capability_snapshot_requires_the_configured_device() {
        let config = GrpcProviderConfig {
            required_device_id: Some("kuavo-mujoco-01".into()),
            ..Default::default()
        };
        let snapshot = validate_capability_facts(&config, capabilities()).unwrap();
        assert_eq!(snapshot.provider_id, "kuavo-bridge");
        assert_eq!(snapshot.device_ids, [DeviceId("kuavo-mujoco-01".into())]);

        let missing = GrpcProviderConfig {
            required_device_id: Some("missing".into()),
            ..Default::default()
        };
        assert!(matches!(
            validate_capability_facts(&missing, capabilities()),
            Err(BridgeStartupError::InvalidCapabilities(reason)) if reason.contains("absent")
        ));
    }

    #[test]
    fn capability_snapshot_rejects_protocol_or_incomplete_facts() {
        let config = GrpcProviderConfig::default();
        let mut mismatch = capabilities();
        mismatch.protocol_version = "2.0".into();
        assert!(validate_capability_facts(&config, mismatch).is_err());

        let mut empty = capabilities();
        empty.device_ids.clear();
        assert!(validate_capability_facts(&config, empty).is_err());

        let mut zero_bytes = capabilities();
        zero_bytes.max_message_bytes = 0;
        assert!(validate_capability_facts(&config, zero_bytes).is_err());

        let mut bad_digest = capabilities();
        bad_digest.protocol_digest = "sha256:other".into();
        assert!(matches!(
            validate_capability_facts(&config, bad_digest),
            Err(BridgeStartupError::ProtocolDigestMismatch { .. })
        ));

        let missing_schema = GrpcProviderConfig {
            required_observation_schemas: vec![ObservationSchemaRequirement {
                schema: "base_twist".into(),
                schema_version: 1,
            }],
            ..Default::default()
        };
        assert!(matches!(
            validate_capability_facts(&missing_schema, capabilities()),
            Err(BridgeStartupError::MissingObservationSchema { .. })
        ));

        let real_profile = GrpcProviderConfig {
            expected_execution_environment: ExecutionEnvironment::Real,
            ..Default::default()
        };
        assert!(matches!(
            validate_capability_facts(&real_profile, capabilities()),
            Err(BridgeStartupError::ExecutionEnvironmentMismatch { .. })
        ));

        let mut incomplete_real = capabilities();
        incomplete_real.execution_environment = wire::ExecutionEnvironment::Real.into();
        assert!(matches!(
            validate_capability_facts(&real_profile, incomplete_real),
            Err(BridgeStartupError::MissingSafetyCapabilities { missing, .. })
                if missing.contains("emergency_stop") && missing.contains("joint_limits")
        ));
    }

    #[test]
    fn health_snapshot_requires_ready_typed_components() {
        let snapshot = validate_health(health()).unwrap();
        assert_eq!(snapshot.state, "HEALTH_STATE_READY");
        assert_eq!(snapshot.components[0].component, "ros_master");

        let mut optional_degraded = health();
        optional_degraded.components[0].state = wire::HealthState::Degraded.into();
        assert_eq!(
            validate_health(optional_degraded).unwrap().components[0].state,
            "HEALTH_STATE_DEGRADED"
        );

        let mut unhealthy = health();
        unhealthy.state = wire::HealthState::Unavailable.into();
        assert!(matches!(
            validate_health(unhealthy),
            Err(BridgeStartupError::Unhealthy { .. })
        ));
    }
}
