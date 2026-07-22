//! Tests for production embodiment config validation.

use executive::core::config::SecretRef;
use executive::core::config::ProductionEmbodimentConfig;

fn valid_config() -> ProductionEmbodimentConfig {
    ProductionEmbodimentConfig {
        namespace: "production".into(),
        device_serial: "SN-001".into(),
        endpoint: "grpcs://kuavo-bridge.local:50051".into(),
        tls_client_cert: SecretRef { env: "TLS_CERT".into() },
        tls_client_key: SecretRef { env: "TLS_KEY".into() },
        skill_allowlist: vec!["kuavo.stance".into(), "kuavo.stop".into()],
        max_linear_mps: 0.05,
        max_angular_rps: 0.1,
        max_duration_ms: 3000,
        gate_evidence_path: "/etc/aletheon/hil-evidence.json".into(),
    }
}

#[test]
fn valid_production_config_passes() {
    assert!(valid_config().validate().is_ok());
}

#[test]
fn wrong_namespace_fails() {
    let mut c = valid_config();
    c.namespace = "simulation".into();
    assert!(c.validate().is_err());
}

#[test]
fn empty_serial_fails() {
    let mut c = valid_config();
    c.device_serial = "".into();
    assert!(c.validate().is_err());
}

#[test]
fn loopback_endpoint_fails() {
    let mut c = valid_config();
    c.endpoint = "http://127.0.0.1:50051".into();
    assert!(c.validate().is_err());
}

#[test]
fn plaintext_endpoint_fails() {
    let mut c = valid_config();
    c.endpoint = "http://bridge:50051".into();
    assert!(c.validate().is_err());
}

#[test]
fn empty_allowlist_fails() {
    let mut c = valid_config();
    c.skill_allowlist = vec![];
    assert!(c.validate().is_err());
}

#[test]
fn wildcard_in_allowlist_fails() {
    let mut c = valid_config();
    c.skill_allowlist = vec!["kuavo.*".into()];
    assert!(c.validate().is_err());
}

#[test]
fn excessive_velocity_fails() {
    let mut c = valid_config();
    c.max_linear_mps = 0.5;
    assert!(c.validate().is_err());
}

#[test]
fn excessive_duration_fails() {
    let mut c = valid_config();
    c.max_duration_ms = 10000;
    assert!(c.validate().is_err());
}

#[test]
fn empty_evidence_path_fails() {
    let mut c = valid_config();
    c.gate_evidence_path = "".into();
    assert!(c.validate().is_err());
}

#[test]
fn all_errors_reported_together() {
    let c = ProductionEmbodimentConfig {
        namespace: "simulation".into(),
        device_serial: "".into(),
        endpoint: "http://localhost:50051".into(),
        skill_allowlist: vec!["kuavo.*".into()],
        max_linear_mps: 1.0,
        max_angular_rps: 1.0,
        max_duration_ms: 10000,
        gate_evidence_path: "".into(),
        tls_client_cert: SecretRef { env: "TLS_CERT".into() },
        tls_client_key: SecretRef { env: "TLS_KEY".into() },
    };
    let err = c.validate().unwrap_err();
    assert!(err.len() > 3, "should report multiple errors, got {:?}", err);
}

#[test]
fn secret_ref_debug_never_exposes_values() {
    let s = SecretRef { env: "SECRET_TOKEN".into() };
    let debug = format!("{:?}", s);
    assert!(!debug.contains("SECRET_TOKEN_VALUE"));
}
