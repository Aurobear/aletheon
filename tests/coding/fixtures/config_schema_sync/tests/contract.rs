use config_schema_sync::{checked_in_schema, Config};

#[test]
fn timeout_default_validation_and_schema_stay_synchronized() {
    let default = Config::default();
    assert_eq!(default.request_timeout_ms, 30_000);

    let invalid = Config {
        request_timeout_ms: 0,
        ..default
    };
    assert_eq!(
        invalid.validate(),
        Err("request_timeout_ms must be positive")
    );

    let schema = checked_in_schema();
    assert!(schema.contains("\"request_timeout_ms\""));
    assert!(schema.contains("\"minimum\": 1"));
    assert!(schema.contains("\"default\": 30000"));
}
