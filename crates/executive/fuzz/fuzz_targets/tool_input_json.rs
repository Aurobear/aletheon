#![no_main]

use fabric::ToolDefinition;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<serde_json::Value>(data);
    if let Ok(definition) = serde_json::from_slice::<ToolDefinition>(data) {
        let _ = definition.input_schema.as_object();
    }
});
