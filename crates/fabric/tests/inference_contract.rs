use fabric::llm_types::{
    canonicalize_tool_definitions, tool_schema_digest, CacheTelemetry, InferenceUsage,
    ToolDefinition,
};
use fabric::types::inference_receipt::{
    InferenceTerminalReceipt, InferenceTerminalStatus, INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
};
use serde_json::json;

fn tool(name: &str, input_schema: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: format!("{name} tool"),
        input_schema,
    }
}

#[test]
fn legacy_usage_aliases_do_not_invent_cache_writes() {
    let usage: InferenceUsage = serde_json::from_value(json!({
        "tokens_in": 10,
        "tokens_out": 2,
        "cache_hit_tokens": 4,
        "cache_miss_tokens": 6
    }))
    .unwrap();

    assert_eq!(usage.total_input_tokens, Some(10));
    assert_eq!(usage.output_tokens, Some(2));
    assert_eq!(usage.cache_read_tokens, Some(4));
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.cache_telemetry, CacheTelemetry::Unknown);
    let serialized = serde_json::to_value(usage).unwrap();
    assert!(serialized.get("cache_hit_tokens").is_none());
    assert!(serialized.get("cache_miss_tokens").is_none());
}

fn terminal(
    status: InferenceTerminalStatus,
    failure_kind: Option<&str>,
) -> InferenceTerminalReceipt {
    InferenceTerminalReceipt {
        schema_version: INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
        inference_id: "inference-1".into(),
        operation_id: "operation-1".into(),
        provider_id: "anthropic".into(),
        model_id: "claude".into(),
        system_prefix_digest: "sha256:system".into(),
        tool_schema_digest: "sha256:tools".into(),
        status,
        usage: InferenceUsage::default(),
        failure_kind: failure_kind.map(str::to_owned),
        prefix_shape_digest: None,
    }
}

#[test]
fn terminal_receipt_validation_preserves_status_semantics() {
    assert!(terminal(InferenceTerminalStatus::Succeeded, None)
        .validate()
        .is_ok());
    assert!(
        terminal(InferenceTerminalStatus::Failed, Some("provider_terminal"))
            .validate()
            .is_ok()
    );
    assert!(terminal(InferenceTerminalStatus::Failed, None)
        .validate()
        .is_err());
    assert!(
        terminal(InferenceTerminalStatus::Succeeded, Some("unknown"))
            .validate()
            .is_err()
    );
    let mut invalid = terminal(InferenceTerminalStatus::Succeeded, None);
    invalid.tool_schema_digest.clear();
    assert!(invalid.validate().is_err());
}

#[test]
fn tool_schema_digest_is_order_and_object_key_independent() {
    let left = vec![
        tool(
            "zeta",
            json!({"type":"object","properties":{"b":{"type":"string"},"a":{"type":"integer"}}}),
        ),
        tool("alpha", json!({})),
    ];
    let right = vec![
        tool("alpha", json!({})),
        tool(
            "zeta",
            json!({"properties":{"a":{"type":"integer"},"b":{"type":"string"}},"type":"object"}),
        ),
    ];

    assert_eq!(
        canonicalize_tool_definitions(&left).unwrap(),
        canonicalize_tool_definitions(&right).unwrap()
    );
    assert_eq!(
        tool_schema_digest(&left).unwrap(),
        tool_schema_digest(&right).unwrap()
    );
}

#[test]
fn array_order_changes_digest_and_duplicate_names_fail() {
    let first = vec![tool("ordered", json!({"enum":["a","b"]}))];
    let second = vec![tool("ordered", json!({"enum":["b","a"]}))];

    assert_ne!(
        tool_schema_digest(&first).unwrap(),
        tool_schema_digest(&second).unwrap()
    );
    assert!(
        canonicalize_tool_definitions(&[tool("dup", json!({})), tool("dup", json!({}))]).is_err()
    );
}

#[test]
fn reported_sets_telemetry_reported() {
    let usage = InferenceUsage::reported(256, 173, Some(156), Some(100), None);
    assert_eq!(usage.cache_telemetry, CacheTelemetry::Reported);
    assert_eq!(usage.total_input_tokens, Some(256));
    assert_eq!(usage.output_tokens, Some(173));
}

#[test]
fn unsupported_sets_telemetry_unsupported_and_zeroes_cache() {
    let usage = InferenceUsage::unsupported(Some(256), Some(173));
    assert_eq!(usage.cache_telemetry, CacheTelemetry::Unsupported);
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_write_tokens, None);
}

#[test]
fn default_telemetry_is_unknown_all_fields_none() {
    let usage = InferenceUsage::default();
    assert_eq!(usage.cache_telemetry, CacheTelemetry::Unknown);
    assert_eq!(usage.total_input_tokens, None);
    assert_eq!(usage.output_tokens, None);
    assert_eq!(usage.uncached_input_tokens, None);
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_write_tokens, None);
}

#[test]
fn reported_preserves_none_not_zero() {
    let usage = InferenceUsage::reported(256, 173, Some(256), None, None);
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.uncached_input_tokens, Some(256));
}

#[test]
fn reported_explicit_zero_read_is_distinct_from_none() {
    let zero = InferenceUsage::reported(256, 173, Some(256), Some(0), None);
    assert_eq!(zero.cache_read_tokens, Some(0));
    let none = InferenceUsage::reported(256, 173, Some(256), None, None);
    assert_ne!(zero.cache_read_tokens, none.cache_read_tokens);
}

#[test]
fn conservation_read_plus_uncached_equals_total_when_all_known() {
    let usage = InferenceUsage::reported(256, 173, Some(156), Some(100), None);
    assert_eq!(
        usage.uncached_input_tokens.unwrap() + usage.cache_read_tokens.unwrap(),
        usage.total_input_tokens.unwrap()
    );
}

#[test]
fn conservation_violation_is_constructible_but_rejected_by_validate() {
    // InferenceUsage can be constructed with read + uncached != total, but
    // validate() must reject that shape as a non-conserved cache report.
    let usage = InferenceUsage::reported(256, 173, Some(300), Some(100), None);
    assert_ne!(
        usage.uncached_input_tokens.unwrap() + usage.cache_read_tokens.unwrap(),
        usage.total_input_tokens.unwrap()
    );
    assert!(usage.validate().is_err());
}

#[test]
fn validate_accepts_conserved_reported() {
    let usage = InferenceUsage::reported(256, 173, Some(156), Some(100), None);
    assert!(usage.validate().is_ok());
}

#[test]
fn validate_accepts_explicit_zero_read() {
    let usage = InferenceUsage::reported(256, 173, Some(256), Some(0), None);
    assert!(usage.validate().is_ok());
}

#[test]
fn validate_accepts_unknown_with_no_figures() {
    assert!(InferenceUsage::default().validate().is_ok());
}

#[test]
fn validate_accepts_missing_figures_kept_none() {
    let usage = InferenceUsage::reported(256, 173, None, None, None);
    assert!(usage.validate().is_ok());
}

#[test]
fn validate_rejects_non_conserved() {
    let usage = InferenceUsage::reported(256, 173, Some(300), Some(100), None);
    assert!(usage.validate().is_err());
}

#[test]
fn validate_rejects_read_exceeding_total() {
    let usage = InferenceUsage::reported(256, 173, Some(0), Some(300), None);
    assert!(usage.validate().is_err());
}

#[test]
fn validate_rejects_unsupported_with_cache_figures() {
    let mut usage = InferenceUsage::unsupported(Some(256), Some(173));
    usage.cache_read_tokens = Some(100);
    assert!(usage.validate().is_err());
}

#[test]
fn canonical_field_names_round_trip_stably() {
    let usage = InferenceUsage::reported(256, 173, Some(156), Some(100), None);
    let json = serde_json::to_value(&usage).unwrap();
    let object = json.as_object().unwrap();
    for key in [
        "total_input_tokens",
        "output_tokens",
        "uncached_input_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "cache_telemetry",
    ] {
        assert!(object.contains_key(key), "missing canonical key {key}");
    }
    let back: InferenceUsage = serde_json::from_value(json).unwrap();
    assert_eq!(back, usage);
}

#[test]
fn canonical_alias_cache_read_tokens_parses() {
    let usage: InferenceUsage = serde_json::from_value(json!({ "cache_read_tokens": 4 })).unwrap();
    assert_eq!(usage.cache_read_tokens, Some(4));
}
