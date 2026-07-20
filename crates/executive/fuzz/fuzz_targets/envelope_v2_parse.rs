#![no_main]

use fabric::ipc::envelope_v2::EnvelopeV2;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // EnvelopeV2 has no bespoke byte decoder. Its public wire contract is
    // serde JSON, so malformed input is exercised through serde_json directly.
    if let Ok(envelope) = serde_json::from_slice::<EnvelopeV2>(data) {
        let _ = envelope.validate_known_schema();
        let _ = serde_json::to_vec(&envelope);
    }
});
