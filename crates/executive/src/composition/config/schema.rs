//! Deterministic JSON Schema generation for the Executive application root.

use super::AppConfig;

pub fn generated_schema_value() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(AppConfig)).expect("AppConfig schema serializes")
}

pub fn generated_schema_json() -> String {
    let mut output = serde_json::to_string_pretty(&generated_schema_value())
        .expect("AppConfig schema serializes");
    output.push('\n');
    output
}
