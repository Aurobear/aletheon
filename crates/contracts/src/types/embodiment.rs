//! Version-stable embodiment protocol DTOs.
//!
//! This boundary intentionally contains no ROS or vendor-specific types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{types::frame::FrameRef, MonoDeadline, MonoTime, OperationId};

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct DeviceId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SkillId(pub String);

/// Provider-attested execution environment. The value comes from the device
/// provider capability handshake; operator configuration may only state the
/// environment it expects and cannot upgrade this fact.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    #[default]
    Simulation,
    Hil,
    Real,
}

impl ExecutionEnvironment {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Simulation => "simulation",
            Self::Hil => "hil",
            Self::Real => "real",
        }
    }
}

/// Bridge/device-owned safety capability facts negotiated before actuation is
/// exposed. Capabilities are deliberately independent booleans: `safe_stop` is
/// an upper-layer request path and never implies a device-local emergency stop.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafetyCapabilityManifest {
    /// Stable hardware identity. Simulation may leave this empty; HIL/real may
    /// not, and Executive matches it to the deployment gate.
    pub device_serial: String,
    pub watchdog: bool,
    pub watchdog_timeout_ms: u64,
    pub heartbeat: bool,
    pub heartbeat_interval_ms: u64,
    pub emergency_stop: bool,
    pub joint_limits: bool,
    pub velocity_limits: bool,
    pub torque_or_current_limits: bool,
    pub control_ownership: bool,
    pub safe_stop: bool,
    /// Device/driver-local stop path that remains available without Aletheon.
    pub independent_hard_stop: bool,
    /// Digest of the bridge/driver-enforced limits, not an operator override.
    pub limits_digest: String,
    /// Bridge-local heartbeat timeout. Zero is invalid when heartbeat support is
    /// advertised.
    pub heartbeat_timeout_ms: u64,
}

impl SafetyCapabilityManifest {
    /// Return stable capability names missing from the selected deployment gate.
    /// Simulation still requires the independent liveness/ownership/safe-stop
    /// boundary. HIL and real execution require the complete manifest.
    pub fn missing_for(&self, environment: ExecutionEnvironment) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for (name, supported) in [
            ("watchdog", self.watchdog),
            ("heartbeat", self.heartbeat),
            ("control_ownership", self.control_ownership),
            ("safe_stop", self.safe_stop),
        ] {
            if !supported {
                missing.push(name);
            }
        }
        if matches!(
            environment,
            ExecutionEnvironment::Hil | ExecutionEnvironment::Real
        ) {
            for (name, supported) in [
                ("emergency_stop", self.emergency_stop),
                ("joint_limits", self.joint_limits),
                ("velocity_limits", self.velocity_limits),
                ("torque_or_current_limits", self.torque_or_current_limits),
            ] {
                if !supported {
                    missing.push(name);
                }
            }
        }
        if self.heartbeat && self.heartbeat_timeout_ms == 0 {
            missing.push("heartbeat_timeout_ms");
        }
        if self.watchdog && self.watchdog_timeout_ms == 0 {
            missing.push("watchdog_timeout_ms");
        }
        if self.heartbeat && self.heartbeat_interval_ms == 0 {
            missing.push("heartbeat_interval_ms");
        }
        if self.heartbeat
            && self.heartbeat_interval_ms > 0
            && self.heartbeat_timeout_ms < self.heartbeat_interval_ms.saturating_mul(2)
        {
            missing.push("heartbeat_timeout_too_short");
        }
        if (self.joint_limits || self.velocity_limits || self.torque_or_current_limits)
            && self.limits_digest.trim().is_empty()
        {
            missing.push("limits_digest");
        }
        if matches!(
            environment,
            ExecutionEnvironment::Hil | ExecutionEnvironment::Real
        ) {
            if self.device_serial.trim().is_empty() {
                missing.push("device_serial");
            }
            if self.limits_digest.trim().is_empty() && !missing.contains(&"limits_digest") {
                missing.push("limits_digest");
            }
        }
        if environment == ExecutionEnvironment::Real && !self.independent_hard_stop {
            missing.push("independent_hard_stop");
        }
        missing
    }
}

/// Canonical SHA-256 digest over the provider-attested environment and safety
/// manifest. Deployment profiles pin this value so a different Bridge build or
/// device capability set cannot silently cross a HIL/real gate.
pub fn safety_capability_manifest_digest(
    environment: ExecutionEnvironment,
    manifest: &SafetyCapabilityManifest,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(environment, manifest))
        .map_err(|error| format!("serialize safety capability manifest: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbodiedObservation {
    pub schema: String,
    pub schema_version: u16,
    pub source: String,
    pub sequence: u64,
    pub source_time: MonoTime,
    pub received_at: MonoTime,
    /// Original cross-machine wall-clock timestamps retained separately from
    /// process-local monotonic scheduling facts.
    pub source_unix_ms: i64,
    pub received_unix_ms: i64,
    pub valid_until: Option<MonoDeadline>,
    pub confidence: f32,
    /// Coordinate/reference frame for non-visual observations (for example
    /// `map`). Visual observations use the typed `frame` contract below.
    pub reference_frame: Option<String>,
    pub frame: Option<FrameRef>,
    pub payload: serde_json::Value,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskClass {
    Read,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillDescriptor {
    pub skill: SkillId,
    pub device: DeviceId,
    pub summary: String,
    pub input_schema: serde_json::Value,
    pub risk: RiskClass,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub preconditions: Vec<String>,
    pub success_criteria: Vec<String>,
}

impl SkillDescriptor {
    /// Validate the complete descriptor contract before it becomes an execution
    /// allowlist entry. Unsupported JSON Schema keywords fail closed instead of
    /// being silently ignored.
    pub fn validate_contract(&self, expected_device: &DeviceId) -> Result<(), String> {
        if self.skill.0.trim().is_empty() {
            return Err("skill id is empty".into());
        }
        if &self.device != expected_device {
            return Err(format!(
                "descriptor device mismatch: expected={} actual={}",
                expected_device.0, self.device.0
            ));
        }
        if self.summary.trim().is_empty() {
            return Err(format!("skill {} summary is empty", self.skill.0));
        }
        if self.timeout_ms == 0 {
            return Err(format!("skill {} timeout_ms must be nonzero", self.skill.0));
        }
        if self.preconditions.is_empty() {
            return Err(format!("skill {} preconditions are empty", self.skill.0));
        }
        if self.success_criteria.is_empty() {
            return Err(format!("skill {} success criteria are empty", self.skill.0));
        }
        crate::adapters::skill_schema::validate_skill_input_schema(&self.input_schema)
            .map_err(|reason| format!("skill {} input schema: {reason}", self.skill.0))
    }

    /// Validate Policy-produced parameters against the startup-validated schema.
    pub fn validate_parameters(&self, parameters: &serde_json::Value) -> Result<(), String> {
        crate::adapters::skill_schema::validate_instance(&self.input_schema, parameters)
            .map_err(|reason| format!("skill {} parameters: {reason}", self.skill.0))
    }
}

/// Deterministic digest over the exact startup allowlist. Descriptors are sorted
/// by device/skill and JSON object keys are canonicalized before hashing.
pub fn skill_descriptor_digest(descriptors: &[SkillDescriptor]) -> Result<String, String> {
    let mut descriptors = descriptors.to_vec();
    descriptors.sort_by(|left, right| {
        (&left.device.0, &left.skill.0).cmp(&(&right.device.0, &right.skill.0))
    });
    let value = serde_json::to_value(descriptors)
        .map_err(|error| format!("serialize skill descriptors: {error}"))?;
    let canonical = canonical_json(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("encode canonical skill descriptors: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Canonical digest used to bind an explicit operator approval to the exact
/// device, skill, and parameters that will be submitted for execution.
pub fn skill_request_digest(request: &SkillRequest) -> Result<String, String> {
    let value = serde_json::to_value(request)
        .map_err(|error| format!("serialize skill request: {error}"))?;
    let canonical = canonical_json(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("encode canonical skill request: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted = map
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<std::collections::BTreeMap<_, _>>();
            serde_json::to_value(sorted).expect("BTreeMap JSON serialization is infallible")
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        other => other,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillRequest {
    pub skill: SkillId,
    pub device: DeviceId,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillProgress {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub fraction: f32,
    pub note: String,
    pub at: MonoTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SkillOutcome {
    Succeeded,
    Failed { reason: String },
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillResult {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub device: DeviceId,
    pub outcome: SkillOutcome,
    pub duration_ms: u64,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillDispatchError {
    #[error("no embodiment provider: {0}")]
    NoProvider(String),
    #[error("embodiment request rejected: {0}")]
    Rejected(String),
}

#[async_trait::async_trait]
pub trait EmbodimentExecutionPort: Send + Sync {
    async fn observe(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, SkillDispatchError>;
    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, SkillDispatchError>;
    async fn list_skills(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, SkillDispatchError>;
    async fn execute_skill(&self, request: SkillRequest)
        -> Result<SkillResult, SkillDispatchError>;
    async fn cancel(&self, operation_id: &OperationId) -> Result<(), SkillDispatchError>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<(), SkillDispatchError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embodied_observation_roundtrips_json() {
        let obs = EmbodiedObservation {
            schema: "pose".into(),
            schema_version: 1,
            source: "sim:bot".into(),
            sequence: 7,
            source_time: MonoTime(100),
            received_at: MonoTime(105),
            source_unix_ms: 1_700_000_000_100,
            received_unix_ms: 1_700_000_000_105,
            valid_until: Some(MonoDeadline::after(MonoTime(105), 500)),
            confidence: 0.9,
            reference_frame: Some("map".into()),
            frame: None,
            payload: serde_json::json!({"x": 1.0, "y": 2.0}),
            evidence: vec![EvidenceRef {
                kind: "rosbag".into(),
                uri: "artifact://b/1".into(),
            }],
        };
        let json = serde_json::to_string(&obs).unwrap();
        let back: EmbodiedObservation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sequence, 7);
        assert_eq!(back.reference_frame.as_deref(), Some("map"));
        assert_eq!(back.source_unix_ms, 1_700_000_000_100);
    }

    #[test]
    fn skill_ids_are_string_newtypes() {
        assert_eq!(DeviceId("bot".into()).0, "bot");
        assert_eq!(SkillId("wave".into()).0, "wave");
    }

    #[test]
    fn real_safety_manifest_fails_closed_on_missing_hard_stop() {
        let complete = SafetyCapabilityManifest {
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
            limits_digest: "limits-sha256".into(),
        };
        assert!(complete.missing_for(ExecutionEnvironment::Real).is_empty());

        let mut incomplete = complete;
        incomplete.independent_hard_stop = false;
        assert_eq!(
            incomplete.missing_for(ExecutionEnvironment::Real),
            ["independent_hard_stop"]
        );
    }

    #[test]
    fn safety_manifest_digest_binds_environment_and_device_facts() {
        let manifest = SafetyCapabilityManifest {
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
            limits_digest: "limits-sha256".into(),
        };
        let first = safety_capability_manifest_digest(ExecutionEnvironment::Real, &manifest)
            .expect("digest");
        assert_eq!(first.len(), 64);
        assert_ne!(
            first,
            safety_capability_manifest_digest(ExecutionEnvironment::Hil, &manifest)
                .expect("digest")
        );

        let mut changed = manifest;
        changed.independent_hard_stop = false;
        assert_ne!(
            first,
            safety_capability_manifest_digest(ExecutionEnvironment::Real, &changed)
                .expect("digest")
        );
    }

    #[test]
    fn operation_id_parser_accepts_uuid_and_rejects_model_text() {
        let id = OperationId::new();
        assert_eq!(id.0.to_string().parse::<OperationId>().unwrap(), id);
        assert!("cancel-latest".parse::<OperationId>().is_err());
    }

    #[test]
    fn skill_request_digest_binds_parameters_independent_of_json_key_order() {
        let first = SkillRequest {
            skill: SkillId("high-risk".into()),
            device: DeviceId("robot".into()),
            parameters: serde_json::json!({"x": 1, "y": 2}),
        };
        let same = SkillRequest {
            parameters: serde_json::json!({"y": 2, "x": 1}),
            ..first.clone()
        };
        let changed = SkillRequest {
            parameters: serde_json::json!({"x": 1, "y": 3}),
            ..first.clone()
        };
        assert_eq!(
            skill_request_digest(&first).unwrap(),
            skill_request_digest(&same).unwrap()
        );
        assert_ne!(
            skill_request_digest(&first).unwrap(),
            skill_request_digest(&changed).unwrap()
        );
    }

    #[test]
    fn descriptor_contract_and_digest_are_order_stable() {
        let descriptor = SkillDescriptor {
            skill: SkillId("stand".into()),
            device: DeviceId("robot-1".into()),
            summary: "stand safely".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"duration_ms": {"type": "integer", "minimum": 1}},
                "required": ["duration_ms"],
                "additionalProperties": false
            }),
            risk: RiskClass::Low,
            timeout_ms: 5_000,
            cancellable: true,
            preconditions: vec!["ready".into()],
            success_criteria: vec!["stable".into()],
        };
        descriptor
            .validate_contract(&DeviceId("robot-1".into()))
            .unwrap();
        descriptor
            .validate_parameters(&serde_json::json!({"duration_ms": 10}))
            .unwrap();
        assert!(descriptor
            .validate_parameters(&serde_json::json!({"duration_ms": 0}))
            .is_err());
        let mut changed = descriptor.clone();
        changed.skill = SkillId("stand.precise".into());
        changed.summary = "stand with a different reviewed contract".into();
        let first = skill_descriptor_digest(&[descriptor.clone(), changed.clone()]).unwrap();
        let second = skill_descriptor_digest(&[changed.clone(), descriptor.clone()]).unwrap();
        assert_eq!(first, second);
        assert_ne!(
            skill_descriptor_digest(&[descriptor]).unwrap(),
            skill_descriptor_digest(&[changed]).unwrap()
        );
    }
}
