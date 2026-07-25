//! Production deployment gate — all checks must pass before Production namespace is available.
//! Simulation defaults always pass. Production requires all fields and valid evidence.

use crate::device::DeviceNamespace;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
const HARD_MAX_LINEAR_MPS_PROD: f64 = 0.1;
#[allow(dead_code)]
const HARD_MAX_ANGULAR_RPS_PROD: f64 = 0.2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentGateInput {
    pub namespace: DeviceNamespace,
    pub device_id: String,
    pub device_serial: String,
    pub endpoint_identity: String,
    pub manifest_digest: String,
    pub limits_digest: String,
    pub evidence_digest: String,
    /// Evidence expiry as unix milliseconds.
    pub evidence_expiry_ms: i64,
    /// Current time for expiry check.
    pub now_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DeploymentGateResult {
    pub passed: bool,
    pub failures: Vec<String>,
}

impl DeploymentGateResult {
    #[allow(dead_code)]
    fn fail(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            failures: vec![reason.into()],
        }
    }
    fn pass() -> Self {
        Self {
            passed: true,
            failures: vec![],
        }
    }
}

/// Validate deployment gate input. Simulation always passes.
pub fn validate_gate(input: &DeploymentGateInput) -> DeploymentGateResult {
    match input.namespace {
        DeviceNamespace::Simulation => DeploymentGateResult::pass(),
        DeviceNamespace::Lab => validate_lab_gate(input),
        DeviceNamespace::Hil => validate_hil_gate(input),
        DeviceNamespace::Production => validate_production_gate(input),
    }
}

fn validate_lab_gate(input: &DeploymentGateInput) -> DeploymentGateResult {
    let mut failures = Vec::new();
    if input.device_id.is_empty() {
        failures.push("device_id required for Lab".into());
    }
    if input.endpoint_identity.is_empty() {
        failures.push("endpoint_identity required for Lab".into());
    }
    if failures.is_empty() {
        DeploymentGateResult::pass()
    } else {
        DeploymentGateResult {
            passed: false,
            failures,
        }
    }
}

fn validate_hil_gate(input: &DeploymentGateInput) -> DeploymentGateResult {
    let mut failures = Vec::new();
    if input.device_id.is_empty() {
        failures.push("device_id required for HIL".into());
    }
    if input.device_serial.is_empty() {
        failures.push("device_serial required for HIL".into());
    }
    if input.endpoint_identity.is_empty() {
        failures.push("endpoint_identity required for HIL".into());
    }
    if input.manifest_digest.is_empty() {
        failures.push("manifest_digest required for HIL".into());
    }
    if input.limits_digest.is_empty() {
        failures.push("limits_digest required for HIL".into());
    }
    if failures.is_empty() {
        DeploymentGateResult::pass()
    } else {
        DeploymentGateResult {
            passed: false,
            failures,
        }
    }
}

fn validate_production_gate(input: &DeploymentGateInput) -> DeploymentGateResult {
    let mut failures = Vec::new();

    // All HIL requirements plus more
    let hil = validate_hil_gate(input);
    failures.extend(hil.failures);

    // Production-specific requirements
    if input.evidence_digest.is_empty() {
        failures.push("evidence_digest required for Production".into());
    }
    if input.now_ms > input.evidence_expiry_ms {
        failures.push(format!(
            "evidence expired: now {} > expiry {}",
            input.now_ms, input.evidence_expiry_ms
        ));
    }
    // Production must use non-loopback endpoint
    if input.endpoint_identity.contains("127.0.0.1")
        || input.endpoint_identity.contains("localhost")
    {
        failures.push("production endpoint must not be loopback".into());
    }
    // Verify namespace/credential match
    if input.namespace != DeviceNamespace::Production {
        failures.push(format!(
            "expected Production namespace, got {:?}",
            input.namespace
        ));
    }

    if failures.is_empty() {
        DeploymentGateResult::pass()
    } else {
        DeploymentGateResult {
            passed: false,
            failures,
        }
    }
}

impl Default for DeploymentGateInput {
    fn default() -> Self {
        Self {
            namespace: DeviceNamespace::Simulation,
            device_id: String::new(),
            device_serial: String::new(),
            endpoint_identity: String::new(),
            manifest_digest: String::new(),
            limits_digest: String::new(),
            evidence_digest: String::new(),
            evidence_expiry_ms: 0,
            now_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input(ns: DeviceNamespace) -> DeploymentGateInput {
        DeploymentGateInput {
            namespace: ns,
            device_id: "kuavo-01".into(),
            device_serial: "SN-001".into(),
            endpoint_identity: "kuavo-bridge.local:50051".into(),
            manifest_digest: "sha256:abc".into(),
            limits_digest: "sha256:def".into(),
            evidence_digest: "sha256:evidence".into(),
            evidence_expiry_ms: 10000,
            now_ms: 5000,
        }
    }

    #[test]
    fn simulation_always_passes() {
        let input = DeploymentGateInput {
            namespace: DeviceNamespace::Simulation,
            ..Default::default()
        };
        assert!(validate_gate(&input).passed);
    }

    #[test]
    fn hil_missing_fields_fail() {
        let input = DeploymentGateInput {
            namespace: DeviceNamespace::Hil,
            ..Default::default()
        };
        assert!(!validate_gate(&input).passed);
    }

    #[test]
    fn complete_hil_passes() {
        assert!(validate_gate(&valid_input(DeviceNamespace::Hil)).passed);
    }

    #[test]
    fn production_without_evidence_fails() {
        let mut input = valid_input(DeviceNamespace::Production);
        input.evidence_digest = "".into();
        assert!(!validate_gate(&input).passed);
    }

    #[test]
    fn production_with_loopback_fails() {
        let mut input = valid_input(DeviceNamespace::Production);
        input.endpoint_identity = "127.0.0.1:50051".into();
        assert!(!validate_gate(&input).passed);
    }

    #[test]
    fn production_expired_evidence_fails() {
        let mut input = valid_input(DeviceNamespace::Production);
        input.evidence_expiry_ms = 1000;
        input.now_ms = 2000;
        assert!(!validate_gate(&input).passed);
    }

    #[test]
    fn valid_production_passes() {
        assert!(validate_gate(&valid_input(DeviceNamespace::Production)).passed);
    }
}
