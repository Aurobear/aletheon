use config_schema_sync::{checked_in_schema, Config};

#[test]
fn typed_timeout_contract_matches_the_checked_in_schema() {
    let config = Config::default();
    assert_eq!(config.request_timeout_ms, 30_000);
    assert!(config.validate().is_ok());
    assert_eq!(
        Config {
            request_timeout_ms: 0,
            ..config
        }
        .validate(),
        Err("request_timeout_ms must be positive")
    );

    let schema = checked_in_schema();
    for fragment in [
        "\"request_timeout_ms\"",
        "\"type\": \"integer\"",
        "\"minimum\": 1",
        "\"default\": 30000",
    ] {
        assert!(schema.contains(fragment), "schema is missing {fragment}");
    }
}
