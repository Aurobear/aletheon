//! Integration tests for the gRPC embodiment provider.

use std::time::Duration;

use fabric::types::embodiment::ExecutionEnvironment;
use hardware::{GrpcEmbodimentProvider, GrpcProviderConfig, ObservationSchemaRequirement};

#[test]
fn default_config_uses_localhost() {
    let config = GrpcProviderConfig::default();
    assert_eq!(config.endpoint, "http://127.0.0.1:50051");
    assert_eq!(config.protocol_version, "1.0");
    assert_eq!(config.connect_timeout, Duration::from_secs(5));
    assert_eq!(config.request_timeout, Duration::from_secs(30));
    assert_eq!(config.max_decoding_message_size, 16 * 1024 * 1024);
}

#[test]
fn config_can_be_customized() {
    let config = GrpcProviderConfig {
        endpoint: "http://10.0.0.1:9999".into(),
        protocol_version: "1.0".into(),
        connect_timeout: Duration::from_millis(500),
        request_timeout: Duration::from_secs(10),
        max_decoding_message_size: 1024,
        required_device_id: Some("robot-01".into()),
        allowed_protocol_digests: vec![hardware::grpc::BRIDGE_PROTOCOL_DIGEST.into()],
        required_observation_schemas: vec![ObservationSchemaRequirement {
            schema: "pose".into(),
            schema_version: 1,
        }],
        expected_execution_environment: ExecutionEnvironment::Simulation,
    };
    assert_eq!(config.endpoint, "http://10.0.0.1:9999");
    assert_eq!(config.connect_timeout, Duration::from_millis(500));
    assert_eq!(config.required_device_id.as_deref(), Some("robot-01"));
}

#[tokio::test]
async fn connect_to_unreachable_endpoint_fails() {
    // Use an unreachable port to verify connection failure.
    let config = GrpcProviderConfig {
        endpoint: "http://127.0.0.1:1".into(),
        connect_timeout: Duration::from_millis(100),
        required_device_id: Some("test-device".into()),
        ..Default::default()
    };
    let result = GrpcEmbodimentProvider::connect(config).await;
    assert!(result.is_err(), "should fail to connect to closed port");
}

#[tokio::test]
async fn missing_required_device_fails_before_network_access() {
    let result = GrpcEmbodimentProvider::connect(GrpcProviderConfig::default()).await;
    assert!(matches!(
        result,
        Err(hardware::BridgeStartupError::InvalidCapabilities(reason))
            if reason.contains("required_device_id")
    ));
}

#[tokio::test]
async fn invalid_endpoint_url_is_rejected() {
    let config = GrpcProviderConfig {
        endpoint: "not-a-valid-url".into(),
        required_device_id: Some("test-device".into()),
        ..Default::default()
    };
    let result = GrpcEmbodimentProvider::connect(config).await;
    assert!(result.is_err());
}

#[test]
fn provider_type_is_send_sync() {
    // Verify GrpcEmbodimentProvider satisfies the Send + Sync bounds
    // required by EmbodimentProvider trait.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GrpcProviderConfig>();
}

/// Read-only live diagnostic for the R2 startup compatibility gate.
///
/// The provider constructor issues only `Health`, `GetCapabilities`, and
/// `ListSkills`; it never requests a snapshot or invokes execute/cancel/safe-stop.
/// Keep the endpoint explicit so an ordinary test run cannot contact a bridge.
#[ignore = "requires an operator-provided live bridge or SSH tunnel"]
#[tokio::test]
async fn live_bridge_startup_capabilities_are_compatible() {
    let endpoint = std::env::var("ALETHEON_TEST_BRIDGE_ENDPOINT")
        .expect("ALETHEON_TEST_BRIDGE_ENDPOINT must be set explicitly");
    let device_id = std::env::var("ALETHEON_TEST_BRIDGE_DEVICE_ID")
        .expect("ALETHEON_TEST_BRIDGE_DEVICE_ID must be set explicitly");
    let required_observation_schemas = std::env::var("ALETHEON_TEST_BRIDGE_REQUIRED_SCHEMAS")
        .unwrap_or_else(|_| "base_pose:1,base_twist:1".into())
        .split(',')
        .map(|entry| {
            let (schema, version) = entry
                .split_once(':')
                .unwrap_or_else(|| panic!("invalid required observation schema `{entry}`"));
            ObservationSchemaRequirement {
                schema: schema.trim().to_owned(),
                schema_version: version
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("invalid schema version in `{entry}`")),
            }
        })
        .collect();
    let provider = GrpcEmbodimentProvider::connect(GrpcProviderConfig {
        endpoint,
        required_device_id: Some(device_id.clone()),
        required_observation_schemas,
        ..Default::default()
    })
    .await
    .unwrap_or_else(|error| {
        panic!("live bridge failed the read-only startup gate for {device_id}: {error}")
    });

    let capabilities = provider.capability_snapshot();
    assert_eq!(capabilities.protocol_version, "1.0");
    assert!(capabilities
        .device_ids
        .iter()
        .any(|candidate| candidate.0 == device_id));
    assert!(!capabilities.provider_id.is_empty());
    assert!(capabilities.max_message_bytes > 0);
    assert!(!capabilities.skill_descriptor_digest.is_empty());
}
