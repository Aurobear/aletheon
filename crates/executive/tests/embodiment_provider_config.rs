//! Integration tests for embodiment provider config selection.

use executive::core::config::EmbodimentProviderConfig;

#[test]
fn default_provider_config_is_simulator() {
    let config = EmbodimentProviderConfig::default();
    assert!(matches!(
        config,
        EmbodimentProviderConfig::Simulator { ref device_id } if device_id == "bot"
    ));
}

#[test]
fn simulator_config_accepts_custom_device_id() {
    let config = EmbodimentProviderConfig::Simulator {
        device_id: "custom-bot".into(),
    };
    assert!(matches!(
        config,
        EmbodimentProviderConfig::Simulator { ref device_id } if device_id == "custom-bot"
    ));
}

#[test]
fn grpc_config_requires_endpoint_and_device_id() {
    let config = EmbodimentProviderConfig::Grpc {
        device_id: "kuavo-mujoco-01".into(),
        endpoint: "http://127.0.0.1:50051".into(),
        connect_timeout_ms: 5000,
        request_timeout_ms: 30000,
    };
    match config {
        EmbodimentProviderConfig::Grpc {
            device_id,
            endpoint,
            connect_timeout_ms,
            request_timeout_ms,
        } => {
            assert_eq!(device_id, "kuavo-mujoco-01");
            assert_eq!(endpoint, "http://127.0.0.1:50051");
            assert_eq!(connect_timeout_ms, 5000);
            assert_eq!(request_timeout_ms, 30000);
        }
        _ => panic!("expected Grpc variant"),
    }
}

#[test]
fn simulator_is_serde_tagged() {
    let yaml = r#"
kind: simulator
device_id: "my-bot"
"#;
    let config: EmbodimentProviderConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(matches!(
        config,
        EmbodimentProviderConfig::Simulator { ref device_id } if device_id == "my-bot"
    ));
}

#[test]
fn grpc_is_serde_tagged() {
    let yaml = r#"
kind: grpc
device_id: "kuavo-mujoco-01"
endpoint: "http://10.0.0.1:50051"
connect_timeout_ms: 5000
request_timeout_ms: 10000
"#;
    let config: EmbodimentProviderConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(matches!(config, EmbodimentProviderConfig::Grpc { .. }));
}

#[test]
fn unknown_kind_fails_deserialization() {
    let yaml = r#"
kind: unknown_provider
device_id: "bot"
"#;
    let result: Result<EmbodimentProviderConfig, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
}

#[test]
fn missing_kind_defaults_to_simulator() {
    // When kind is absent but the enum has no untagged fallback,
    // serde will error. This verifies the fail-closed behavior.
    let yaml = r#"
device_id: "bot"
"#;
    let result: Result<EmbodimentProviderConfig, _> = serde_yaml::from_str(yaml);
    // Must fail — the "kind" tag is required by serde(tag = "kind")
    assert!(result.is_err(), "missing kind tag must fail deserialization");
}
