//! Typed Robot/Policy configuration and semantic resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{EmbodimentProviderConfig, IntegrationsConfig};

const MIN_CONNECT_TIMEOUT_MS: u64 = 1;
const MAX_CONNECT_TIMEOUT_MS: u64 = 60_000;
const MIN_REQUEST_TIMEOUT_MS: u64 = 1;
const MAX_REQUEST_TIMEOUT_MS: u64 = 300_000;
const MIN_POLL_INTERVAL_MS: u64 = 10;
const MAX_POLL_INTERVAL_MS: u64 = 60_000;
const MAX_PROPOSALS: usize = 64;
const MAX_PERCEPTION_DEVICES: usize = 128;
const MAX_PERCEPTION_FRAMES: usize = 4;
const MAX_CACHED_FRAMES: usize = 1_024;
const MAX_FRAME_AGE_MS: u64 = 60_000;
const MAX_PERCEPTION_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RETRIES: u32 = 1;
const MAX_REPLANS: u32 = 1;

/// Operator-owned optional Robot capability configuration. Presence enables a
/// per-turn Robot target; absence never changes the General default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RobotIntegrationConfig {
    /// Scene identity recorded on every episode (for example
    /// `kuavo-mujoco/default-v40`).
    pub scene_version: String,
    /// Deployment gate expected from the provider-owned capability handshake.
    /// This value never upgrades the Bridge's attested environment.
    pub execution_environment: ::contracts::types::embodiment::ExecutionEnvironment,
    /// Pinned device and reviewed evidence for HIL/real deployments. Simulation
    /// must omit this section; HIL and real must provide it.
    pub deployment_gate: Option<RobotDeploymentGateConfig>,
    /// A Robot capability must name a real Policy gateway explicitly.
    pub policy: Option<RobotPolicyConfig>,
    /// Perception polling and world-state bounds.
    pub perception: RobotPerceptionConfig,
    /// Observation schemas the Bridge must advertise before Robot startup.
    #[serde(default = "default_required_observation_schemas")]
    pub required_observation_schemas: Vec<RobotObservationSchemaConfig>,
    /// Bounded retry count consumed by `RobotHarness`.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Bounded replan count consumed by `RobotHarness`.
    #[serde(default = "default_max_replans")]
    pub max_replans: u32,
}

impl Default for RobotIntegrationConfig {
    fn default() -> Self {
        Self {
            scene_version: String::new(),
            execution_environment: ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            deployment_gate: None,
            policy: None,
            perception: RobotPerceptionConfig::default(),
            required_observation_schemas: default_required_observation_schemas(),
            max_retries: default_max_retries(),
            max_replans: default_max_replans(),
        }
    }
}

/// Operator-reviewed deployment facts. The serial and digests are compared to
/// the provider-attested startup snapshot before the provider is registered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RobotDeploymentGateConfig {
    pub device_serial: String,
    pub safety_manifest_digest: String,
    pub limits_digest: String,
    /// Required only for a real deployment and expected to identify reviewed
    /// HIL/real acceptance evidence.
    pub evidence_digest: String,
    /// Unix milliseconds. Required and checked at bootstrap for real devices.
    pub evidence_expiry_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RobotObservationSchemaConfig {
    pub schema: String,
    pub schema_version: u32,
}

/// Policy gateway settings. Milliseconds remain wire/config values; resolution
/// converts them into checked, bounded `Duration`s before bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RobotPolicyConfig {
    pub endpoint: String,
    pub protocol_version: String,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_policy_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_max_proposals")]
    pub max_proposals: usize,
}

impl Default for RobotPolicyConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            protocol_version: String::new(),
            connect_timeout_ms: default_connect_timeout_ms(),
            request_timeout_ms: default_policy_request_timeout_ms(),
            max_proposals: default_max_proposals(),
        }
    }
}

/// World-state polling settings used by the Robot composition root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RobotPerceptionConfig {
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_max_devices")]
    pub max_devices: usize,
    #[serde(default = "default_max_frame_age_ms")]
    pub max_frame_age_ms: u64,
    #[serde(default = "default_max_frames")]
    pub max_frames: usize,
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: u64,
    #[serde(default = "default_max_cached_frames")]
    pub max_cached_frames_per_device: usize,
    #[serde(default = "default_allowed_uri_prefixes")]
    pub allowed_uri_prefixes: Vec<String>,
    /// Skills absent from this map may run without perception. Listed skills
    /// fail closed unless every schema/version requirement has a selected frame.
    #[serde(default)]
    pub required_by_skill: BTreeMap<String, Vec<RobotObservationSchemaConfig>>,
}

impl Default for RobotPerceptionConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval_ms(),
            max_devices: default_max_devices(),
            max_frame_age_ms: default_max_frame_age_ms(),
            max_frames: default_max_frames(),
            max_total_bytes: default_max_total_bytes(),
            max_cached_frames_per_device: default_max_cached_frames(),
            allowed_uri_prefixes: default_allowed_uri_prefixes(),
            required_by_skill: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRobotPolicyConfig {
    pub endpoint: String,
    pub protocol_version: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_proposals: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRobotPerceptionConfig {
    pub poll_interval: Duration,
    pub max_devices: usize,
    pub max_frame_age: Duration,
    pub max_frames: usize,
    pub max_total_bytes: u64,
    pub max_cached_frames_per_device: usize,
    pub allowed_uri_prefixes: Vec<String>,
    pub required_by_skill: BTreeMap<::contracts::types::embodiment::SkillId, Vec<(String, u16)>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRobotDeploymentGateConfig {
    pub device_serial: String,
    pub safety_manifest_digest: String,
    pub limits_digest: String,
    pub evidence_digest: String,
    pub evidence_expiry_unix_ms: i64,
}

/// Semantic Robot settings passed into daemon composition. Build-owned facts
/// cannot be spoofed by TOML/environment overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRobotIntegrationConfig {
    pub device_id: String,
    pub scene_version: String,
    pub execution_environment: ::contracts::types::embodiment::ExecutionEnvironment,
    pub deployment_gate: Option<ResolvedRobotDeploymentGateConfig>,
    pub aletheon_version: String,
    pub bridge_protocol_digest: String,
    pub required_observation_schemas: Vec<hardware::ObservationSchemaRequirement>,
    pub policy: ResolvedRobotPolicyConfig,
    pub perception: ResolvedRobotPerceptionConfig,
    pub max_retries: u32,
    pub max_replans: u32,
}

impl IntegrationsConfig {
    /// Resolve Robot settings after all application layers have merged.
    ///
    /// `harness_kind` is retained only for migration diagnostics. A complete
    /// Robot integration enables an optional per-turn capability regardless of
    /// the legacy global value; an absent integration never prevents General
    /// turns from starting.
    pub fn resolve_robot(
        &self,
        _harness_kind: cognit::harness::HarnessKind,
    ) -> Result<Option<ResolvedRobotIntegrationConfig>> {
        let Some(robot) = self.robot.as_ref() else {
            return Ok(None);
        };

        let provider = self.embodiment.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "integrations.robot requires an explicit integrations.embodiment provider"
            )
        })?;
        provider.validate_runtime()?;
        let device_id = provider.device_id().to_owned();
        if matches!(provider, EmbodimentProviderConfig::Simulator { .. })
            && robot.execution_environment
                != ::contracts::types::embodiment::ExecutionEnvironment::Simulation
        {
            bail!(
                "integrations.embodiment.kind=simulator can only use integrations.robot.execution_environment=simulation"
            );
        }
        let deployment_gate = resolve_deployment_gate(robot, provider)?;

        let scene_version = non_empty(&robot.scene_version, "integrations.robot.scene_version")?;
        let policy = robot.policy.as_ref().ok_or_else(|| {
            anyhow::anyhow!("HarnessKind::Robot requires integrations.robot.policy")
        })?;
        validate_grpc_endpoint(&policy.endpoint, "integrations.robot.policy.endpoint")?;
        cognit::validate_policy_endpoint(&policy.endpoint)
            .map_err(anyhow::Error::msg)
            .context("validate integrations.robot.policy.endpoint")?;
        let protocol_version = non_empty(
            &policy.protocol_version,
            "integrations.robot.policy.protocol_version",
        )?;
        let connect_timeout = checked_duration_ms(
            policy.connect_timeout_ms,
            MIN_CONNECT_TIMEOUT_MS,
            MAX_CONNECT_TIMEOUT_MS,
            "integrations.robot.policy.connect_timeout_ms",
        )?;
        let request_timeout = checked_duration_ms(
            policy.request_timeout_ms,
            MIN_REQUEST_TIMEOUT_MS,
            MAX_REQUEST_TIMEOUT_MS,
            "integrations.robot.policy.request_timeout_ms",
        )?;
        if policy.max_proposals == 0 || policy.max_proposals > MAX_PROPOSALS {
            bail!("integrations.robot.policy.max_proposals must be within 1..={MAX_PROPOSALS}");
        }
        let poll_interval = checked_duration_ms(
            robot.perception.poll_interval_ms,
            MIN_POLL_INTERVAL_MS,
            MAX_POLL_INTERVAL_MS,
            "integrations.robot.perception.poll_interval_ms",
        )?;
        if robot.perception.max_devices == 0
            || robot.perception.max_devices > MAX_PERCEPTION_DEVICES
        {
            bail!(
                "integrations.robot.perception.max_devices must be within 1..={MAX_PERCEPTION_DEVICES}"
            );
        }
        let max_frame_age = checked_duration_ms(
            robot.perception.max_frame_age_ms,
            1,
            MAX_FRAME_AGE_MS,
            "integrations.robot.perception.max_frame_age_ms",
        )?;
        if robot.perception.max_frames == 0 || robot.perception.max_frames > MAX_PERCEPTION_FRAMES {
            bail!(
                "integrations.robot.perception.max_frames must be within 1..={MAX_PERCEPTION_FRAMES}"
            );
        }
        if robot.perception.max_total_bytes == 0
            || robot.perception.max_total_bytes > MAX_PERCEPTION_BYTES
        {
            bail!(
                "integrations.robot.perception.max_total_bytes must be within 1..={MAX_PERCEPTION_BYTES}"
            );
        }
        if robot.perception.max_cached_frames_per_device < robot.perception.max_frames
            || robot.perception.max_cached_frames_per_device > MAX_CACHED_FRAMES
        {
            bail!(
                "integrations.robot.perception.max_cached_frames_per_device must be within max_frames..={MAX_CACHED_FRAMES}"
            );
        }
        let mut allowed_uri_prefixes = robot.perception.allowed_uri_prefixes.clone();
        if allowed_uri_prefixes.is_empty() {
            bail!("integrations.robot.perception.allowed_uri_prefixes must not be empty");
        }
        for prefix in &allowed_uri_prefixes {
            let workspace_prefix = prefix.strip_prefix("workspace://");
            if prefix != "artifact://sha256/"
                && !workspace_prefix.is_some_and(|path| {
                    !path.is_empty()
                        && path.ends_with('/')
                        && !path.starts_with('/')
                        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
                })
            {
                bail!(
                    "integrations.robot.perception.allowed_uri_prefixes contains unsupported prefix '{prefix}'"
                );
            }
        }
        allowed_uri_prefixes.sort();
        let prefix_count = allowed_uri_prefixes.len();
        allowed_uri_prefixes.dedup();
        if prefix_count != allowed_uri_prefixes.len() {
            bail!("integrations.robot.perception.allowed_uri_prefixes contains duplicates");
        }
        let mut required_by_skill = BTreeMap::new();
        for (skill, requirements) in &robot.perception.required_by_skill {
            let skill = non_empty(skill, "integrations.robot.perception.required_by_skill key")?;
            if requirements.is_empty() {
                bail!("integrations.robot.perception.required_by_skill.{skill} must not be empty");
            }
            let mut resolved = BTreeSet::new();
            for requirement in requirements {
                let schema = non_empty(
                    &requirement.schema,
                    "integrations.robot.perception.required_by_skill[].schema",
                )?;
                let version = u16::try_from(requirement.schema_version).map_err(|_| {
                    anyhow::anyhow!(
                        "integrations.robot.perception.required_by_skill[].schema_version exceeds u16"
                    )
                })?;
                if version == 0 || !resolved.insert((schema, version)) {
                    bail!(
                        "integrations.robot.perception.required_by_skill.{skill} contains an invalid or duplicate schema"
                    );
                }
            }
            required_by_skill.insert(
                ::contracts::types::embodiment::SkillId(skill),
                resolved.into_iter().collect(),
            );
        }
        if robot.max_retries > MAX_RETRIES {
            bail!("integrations.robot.max_retries must be within 0..={MAX_RETRIES}");
        }
        if robot.max_replans > MAX_REPLANS {
            bail!("integrations.robot.max_replans must be within 0..={MAX_REPLANS}");
        }
        let bridge_protocol_digest = hardware::grpc::BRIDGE_PROTOCOL_DIGEST;
        if bridge_protocol_digest.trim().is_empty() {
            bail!("compiled bridge protocol digest is empty");
        }
        if robot.required_observation_schemas.is_empty() {
            bail!("integrations.robot.required_observation_schemas must not be empty");
        }
        let mut required_observation_schemas = robot
            .required_observation_schemas
            .iter()
            .map(|descriptor| {
                Ok(hardware::ObservationSchemaRequirement {
                    schema: non_empty(
                        &descriptor.schema,
                        "integrations.robot.required_observation_schemas[].schema",
                    )?,
                    schema_version: descriptor.schema_version,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if required_observation_schemas
            .iter()
            .any(|descriptor| descriptor.schema_version == 0)
        {
            bail!(
                "integrations.robot.required_observation_schemas[].schema_version must be nonzero"
            );
        }
        required_observation_schemas.sort();
        let before_dedup = required_observation_schemas.len();
        required_observation_schemas.dedup();
        if required_observation_schemas.len() != before_dedup {
            bail!("integrations.robot.required_observation_schemas contains duplicates");
        }

        Ok(Some(ResolvedRobotIntegrationConfig {
            device_id,
            scene_version,
            execution_environment: robot.execution_environment,
            deployment_gate,
            aletheon_version: env!("CARGO_PKG_VERSION").to_owned(),
            bridge_protocol_digest: bridge_protocol_digest.to_owned(),
            required_observation_schemas,
            policy: ResolvedRobotPolicyConfig {
                endpoint: policy.endpoint.clone(),
                protocol_version,
                connect_timeout,
                request_timeout,
                max_proposals: policy.max_proposals,
            },
            perception: ResolvedRobotPerceptionConfig {
                poll_interval,
                max_devices: robot.perception.max_devices,
                max_frame_age,
                max_frames: robot.perception.max_frames,
                max_total_bytes: robot.perception.max_total_bytes,
                max_cached_frames_per_device: robot.perception.max_cached_frames_per_device,
                allowed_uri_prefixes,
                required_by_skill,
            },
            max_retries: robot.max_retries,
            max_replans: robot.max_replans,
        }))
    }
}

fn resolve_deployment_gate(
    robot: &RobotIntegrationConfig,
    provider: &EmbodimentProviderConfig,
) -> Result<Option<ResolvedRobotDeploymentGateConfig>> {
    use ::contracts::types::embodiment::ExecutionEnvironment;

    if robot.execution_environment == ExecutionEnvironment::Simulation {
        if robot.deployment_gate.is_some() {
            bail!(
                "integrations.robot.deployment_gate must be omitted for execution_environment=simulation"
            );
        }
        return Ok(None);
    }

    let EmbodimentProviderConfig::Grpc { endpoint, .. } = provider else {
        bail!("HIL/real deployment gates require integrations.embodiment.kind=grpc");
    };
    let gate = robot.deployment_gate.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "integrations.robot.deployment_gate is required for execution_environment={}",
            robot.execution_environment.as_str()
        )
    })?;
    let device_serial = non_empty(
        &gate.device_serial,
        "integrations.robot.deployment_gate.device_serial",
    )?;
    let safety_manifest_digest = checked_sha256(
        &gate.safety_manifest_digest,
        "integrations.robot.deployment_gate.safety_manifest_digest",
    )?;
    let limits_digest = checked_sha256(
        &gate.limits_digest,
        "integrations.robot.deployment_gate.limits_digest",
    )?;

    let (evidence_digest, evidence_expiry_unix_ms) = if robot.execution_environment
        == ExecutionEnvironment::Real
    {
        validate_real_embodiment_endpoint(endpoint)?;
        let digest = checked_sha256(
            &gate.evidence_digest,
            "integrations.robot.deployment_gate.evidence_digest",
        )?;
        if gate.evidence_expiry_unix_ms <= 0 {
            bail!(
                    "integrations.robot.deployment_gate.evidence_expiry_unix_ms must be positive for execution_environment=real"
                );
        }
        (digest, gate.evidence_expiry_unix_ms)
    } else {
        let digest = if gate.evidence_digest.trim().is_empty() {
            String::new()
        } else {
            checked_sha256(
                &gate.evidence_digest,
                "integrations.robot.deployment_gate.evidence_digest",
            )?
        };
        (digest, gate.evidence_expiry_unix_ms)
    };

    Ok(Some(ResolvedRobotDeploymentGateConfig {
        device_serial,
        safety_manifest_digest,
        limits_digest,
        evidence_digest,
        evidence_expiry_unix_ms,
    }))
}

impl EmbodimentProviderConfig {
    pub fn device_id(&self) -> &str {
        match self {
            Self::Simulator { device_id } | Self::Grpc { device_id, .. } => device_id,
        }
    }

    /// Validate provider semantics and all configured durations without opening
    /// a socket. Bootstrap calls this again at the network boundary.
    pub fn validate_runtime(&self) -> Result<()> {
        non_empty(self.device_id(), "integrations.embodiment.device_id")?;
        if let Self::Grpc {
            endpoint,
            connect_timeout_ms,
            request_timeout_ms,
            ..
        } = self
        {
            validate_grpc_endpoint(endpoint, "integrations.embodiment.endpoint")?;
            checked_duration_ms(
                *connect_timeout_ms,
                MIN_CONNECT_TIMEOUT_MS,
                MAX_CONNECT_TIMEOUT_MS,
                "integrations.embodiment.connect_timeout_ms",
            )?;
            checked_duration_ms(
                *request_timeout_ms,
                MIN_REQUEST_TIMEOUT_MS,
                MAX_REQUEST_TIMEOUT_MS,
                "integrations.embodiment.request_timeout_ms",
            )?;
        }
        Ok(())
    }

    pub fn checked_grpc_timeouts(&self) -> Result<Option<(Duration, Duration)>> {
        self.validate_runtime()?;
        let Self::Grpc {
            connect_timeout_ms,
            request_timeout_ms,
            ..
        } = self
        else {
            return Ok(None);
        };
        Ok(Some((
            checked_duration_ms(
                *connect_timeout_ms,
                MIN_CONNECT_TIMEOUT_MS,
                MAX_CONNECT_TIMEOUT_MS,
                "integrations.embodiment.connect_timeout_ms",
            )?,
            checked_duration_ms(
                *request_timeout_ms,
                MIN_REQUEST_TIMEOUT_MS,
                MAX_REQUEST_TIMEOUT_MS,
                "integrations.embodiment.request_timeout_ms",
            )?,
        )))
    }
}

fn checked_duration_ms(value: u64, min: u64, max: u64, path: &str) -> Result<Duration> {
    if value < min || value > max {
        bail!("{path} must be within {min}..={max} milliseconds");
    }
    let seconds = value / 1_000;
    let milliseconds = value % 1_000;
    Duration::from_secs(seconds)
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(|| anyhow::anyhow!("{path} overflows std::time::Duration"))
}

fn non_empty(value: &str, path: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{path} must not be empty");
    }
    Ok(value.to_owned())
}

fn checked_sha256(value: &str, path: &str) -> Result<String> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{path} must be a 64-character SHA-256 hex digest");
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_real_embodiment_endpoint(endpoint: &str) -> Result<()> {
    let (scheme, remainder) = endpoint
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("integrations.embodiment.endpoint must include a scheme"))?;
    if !matches!(scheme, "https" | "grpcs") {
        bail!("execution_environment=real requires a TLS embodiment endpoint");
    }
    let authority = remainder.split('/').next().unwrap_or_default();
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .unwrap_or_default()
    } else {
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
    };
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if loopback {
        bail!("execution_environment=real requires a non-loopback embodiment endpoint");
    }
    Ok(())
}

/// Accept TLS endpoints anywhere and plaintext only on an exact loopback host.
/// Userinfo/query/fragment are rejected so config diagnostics can safely show
/// the effective endpoint without leaking URI-embedded credentials.
fn validate_grpc_endpoint(endpoint: &str, path: &str) -> Result<()> {
    if endpoint.trim() != endpoint || endpoint.is_empty() {
        bail!("{path} must be a non-empty endpoint without surrounding whitespace");
    }
    if endpoint.contains('@') || endpoint.contains('?') || endpoint.contains('#') {
        bail!("{path} must not contain userinfo, query parameters, or fragments");
    }
    let (scheme, remainder) = endpoint
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("{path} must include an explicit URL scheme"))?;
    if !matches!(scheme, "http" | "https" | "grpcs") {
        bail!("{path} must use http, https, or grpcs");
    }
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        bail!("{path} must include a host");
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .ok_or_else(|| anyhow::anyhow!("{path} contains an invalid IPv6 host"))?
    } else {
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
    };
    if host.is_empty() {
        bail!("{path} must include a host");
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if scheme == "http" && !loopback {
        bail!("{path} rejects plaintext gRPC for non-loopback host '{host}'");
    }
    Ok(())
}

fn default_connect_timeout_ms() -> u64 {
    5_000
}

fn default_policy_request_timeout_ms() -> u64 {
    30_000
}

fn default_max_proposals() -> usize {
    4
}

fn default_poll_interval_ms() -> u64 {
    250
}

fn default_max_devices() -> usize {
    16
}

fn default_max_frame_age_ms() -> u64 {
    2_000
}

fn default_max_frames() -> usize {
    4
}

fn default_max_total_bytes() -> u64 {
    16 * 1024 * 1024
}

fn default_max_cached_frames() -> usize {
    64
}

fn default_allowed_uri_prefixes() -> Vec<String> {
    vec!["artifact://sha256/".into()]
}

fn default_max_retries() -> u32 {
    1
}

fn default_max_replans() -> u32 {
    1
}

fn default_required_observation_schemas() -> Vec<RobotObservationSchemaConfig> {
    vec![
        RobotObservationSchemaConfig {
            schema: "base_pose".into(),
            schema_version: 1,
        },
        RobotObservationSchemaConfig {
            schema: "base_twist".into(),
            schema_version: 1,
        },
    ]
}
