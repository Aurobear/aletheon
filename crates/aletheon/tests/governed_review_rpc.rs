use aletheon::config::{AppConfig, GovernedReviewSettings};
use aletheon::wiring::governed_review::{GovernedReviewLimits, ReviewCapabilities};

#[test]
fn rpc_dispatch_exposes_exact_governed_lifecycle_methods() {
    let dispatch = include_str!("../src/wiring/daemon/handler/rpc.rs");
    for method in [
        "review.capabilities",
        "review.submit",
        "review.status",
        "review.wait",
        "review.cancel",
    ] {
        assert_eq!(dispatch.matches(&format!("\"{method}\"")).count(), 1);
    }
    let handler = include_str!("../src/wiring/daemon/handler/rpc/rpc_review.rs");
    assert!(handler.contains("connection.principal_id"));
    assert!(handler.contains("receipt.status.is_terminal()"));
    assert!(handler.contains("INCOMPATIBLE_SCHEMA: i64 = -32044"));
    assert!(handler.contains("IDEMPOTENCY_CONFLICT: i64 = -32045"));
}

#[test]
fn capabilities_are_versioned_generic_and_bounded() {
    let capabilities = ReviewCapabilities {
        protocol_version: 1,
        schema_versions: vec![2, 1],
        capabilities: vec!["terminal_wait".into(), "idempotent_submission".into()],
        limits: GovernedReviewLimits::default(),
        daemon_instance_id: "daemon-instance".into(),
        daemon_started_at_unix_ms: 1,
        model_spec: "default".into(),
    };
    let json = serde_json::to_value(capabilities).unwrap();
    assert_eq!(json["schema_versions"], serde_json::json!([2, 1]));
    assert_eq!(json["limits"]["max_pending_jobs"], 64);
    assert_eq!(json["daemon_instance_id"], "daemon-instance");
}

#[test]
fn typed_config_defaults_enabled_and_rejects_tool_authority() {
    let config: AppConfig = toml::from_str("").unwrap();
    assert!(config.governed_review.enabled);
    assert_eq!(config.governed_review.max_tool_calls, 0);
    config.governed_review.validate().unwrap();

    let invalid = GovernedReviewSettings {
        max_tool_calls: 1,
        ..GovernedReviewSettings::default()
    };
    assert!(invalid.validate().is_err());
}
