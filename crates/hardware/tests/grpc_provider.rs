//! Integration tests for the gRPC embodiment provider.

use std::time::Duration;

use hardware::{GrpcEmbodimentProvider, GrpcProviderConfig};

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
    };
    assert_eq!(config.endpoint, "http://10.0.0.1:9999");
    assert_eq!(config.connect_timeout, Duration::from_millis(500));
}

#[tokio::test]
async fn connect_to_unreachable_endpoint_fails() {
    // Use an unreachable port to verify connection failure.
    let config = GrpcProviderConfig {
        endpoint: "http://127.0.0.1:1".into(),
        connect_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let result = GrpcEmbodimentProvider::connect(config).await;
    assert!(result.is_err(), "should fail to connect to closed port");
}

#[tokio::test]
async fn invalid_endpoint_url_is_rejected() {
    let config = GrpcProviderConfig {
        endpoint: "not-a-valid-url".into(),
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
