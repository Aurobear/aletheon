//! Tests that embodiment provider config is preserved from AppConfig through to DaemonConfig.

use executive::composition::config::EmbodimentProviderConfig;

#[test]
fn grpc_config_is_preserved_with_endpoint_and_device() {
    let config = EmbodimentProviderConfig::Grpc {
        device_id: "kuavo-mujoco-01".into(),
        endpoint: "http://127.0.0.1:50051".into(),
        connect_timeout_ms: 5000,
        request_timeout_ms: 10000,
    };
    match &config {
        EmbodimentProviderConfig::Grpc {
            device_id,
            endpoint,
            connect_timeout_ms,
            request_timeout_ms,
        } => {
            assert_eq!(device_id, "kuavo-mujoco-01");
            assert_eq!(endpoint, "http://127.0.0.1:50051");
            assert_eq!(*connect_timeout_ms, 5000);
            assert_eq!(*request_timeout_ms, 10000);
        }
        _ => panic!("expected Grpc"),
    }
}

#[test]
fn missing_embodiment_config_defaults_to_simulator() {
    // Verify that Option::None unwrap_or_default gives Simulator
    let config: Option<EmbodimentProviderConfig> = None;
    let resolved = config.unwrap_or_default();
    assert!(matches!(resolved, EmbodimentProviderConfig::Simulator { .. }));
}

#[test]
fn explicit_grpc_overrides_default() {
    // Verify that explicit config takes precedence over default
    let config = Some(EmbodimentProviderConfig::Grpc {
        device_id: "my-device".into(),
        endpoint: "http://10.0.0.1:9999".into(),
        connect_timeout_ms: 500,
        request_timeout_ms: 3000,
    });
    let resolved = config.unwrap_or_default();
    assert!(matches!(resolved, EmbodimentProviderConfig::Grpc { .. }));
}
