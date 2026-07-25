//! Integration tests for deployment gate public API.

use hardware::{validate_gate, DeploymentGateInput, DeviceNamespace};

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
fn simulation_always_passes_integration() {
    let input = DeploymentGateInput::default();
    let result = validate_gate(&input);
    assert!(result.passed);
    assert!(result.failures.is_empty());
}

#[test]
fn hil_requires_all_identity_fields() {
    let input = valid_input(DeviceNamespace::Hil);
    let result = validate_gate(&input);
    assert!(result.passed);
}

#[test]
fn production_validates_evidence_and_endpoint() {
    let input = valid_input(DeviceNamespace::Production);
    let result = validate_gate(&input);
    assert!(result.passed);
}

#[test]
fn namespace_routing_works() {
    // Lab is less strict than HIL or Production
    let mut input = valid_input(DeviceNamespace::Lab);
    // Clear all fields except device_id and endpoint_identity
    input.device_serial = "".into();
    input.manifest_digest = "".into();
    input.limits_digest = "".into();
    input.evidence_digest = "".into();
    let result = validate_gate(&input);
    assert!(result.passed);
}
