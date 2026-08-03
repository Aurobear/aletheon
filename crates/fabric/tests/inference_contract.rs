use fabric::llm_types::{
    canonicalize_tool_definitions, tool_schema_digest, CacheTelemetry, InferenceUsage,
    ToolDefinition,
};
use fabric::{
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
