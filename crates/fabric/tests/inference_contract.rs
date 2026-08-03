use fabric::llm_types::{
    canonicalize_tool_definitions, tool_schema_digest, ToolDefinition,
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
    assert!(canonicalize_tool_definitions(&[
        tool("dup", json!({})),
        tool("dup", json!({}))
    ])
    .is_err());
}
