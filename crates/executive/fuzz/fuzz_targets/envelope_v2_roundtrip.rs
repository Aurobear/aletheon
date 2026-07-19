#![no_main]

use arbitrary::Arbitrary;
use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use fabric::NamespaceId;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct EnvelopeInput {
    schema: String,
    source: String,
    target: String,
    namespace: String,
    pattern: u8,
    logical_time: u64,
    priority: u8,
    payload: String,
}

fuzz_target!(|input: EnvelopeInput| {
    let pattern = match input.pattern % 3 {
        0 => DeliveryPattern::Direct,
        1 => DeliveryPattern::FanOut,
        _ => DeliveryPattern::RequestResponse,
    };
    let envelope = EnvelopeV2::new(
        SchemaId::from(input.schema),
        Target::from(input.source),
        Target::from(input.target),
        pattern,
        NamespaceId(input.namespace),
        serde_json::Value::String(input.payload),
    )
    .with_logical_time(input.logical_time)
    .with_priority(input.priority);

    let encoded = serde_json::to_vec(&envelope).expect("EnvelopeV2 must serialize");
    let decoded: EnvelopeV2 =
        serde_json::from_slice(&encoded).expect("serialized EnvelopeV2 must deserialize");
    assert_eq!(decoded, envelope);
});
