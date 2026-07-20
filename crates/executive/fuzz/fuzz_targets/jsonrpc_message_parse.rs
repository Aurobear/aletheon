#![no_main]

use fabric::protocol::client::{ClientEvent, ClientMessage};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    // Exercise the compatibility JSON-RPC parser with arbitrary bytes. Invalid
    // UTF-8 and invalid JSON are normal inputs, not failures.
    if let Ok(value) = serde_json::from_slice::<Value>(data) {
        if let Some(object) = value.as_object() {
            let _ = object.get("jsonrpc").and_then(Value::as_str);
            let _ = object.get("id");
            let _ = object.get("method").and_then(Value::as_str);
            let _ = object.get("params");
        }

        // A successfully parsed value must remain valid JSON after encoding.
        if let Ok(encoded) = serde_json::to_vec(&value) {
            let _: Value = serde_json::from_slice(&encoded).expect("Value must round-trip");
        }
    }

    // Also drive the real, versioned Fabric client protocol decoder. This
    // covers its tagged event payloads and forward-compatible extensions.
    if let Ok(message) = serde_json::from_slice::<ClientMessage<ClientEvent>>(data) {
        if let Ok(encoded) = serde_json::to_vec(&message) {
            let decoded: ClientMessage<ClientEvent> =
                serde_json::from_slice(&encoded).expect("typed message must round-trip");
            assert_eq!(decoded, message);
        }
    }
});
