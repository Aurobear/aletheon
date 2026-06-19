// crates/aletheon-comm/src/core/envelope.rs

//! Convenience methods and re-exports for Envelope.

pub use aletheon_abi::envelope::*;

use aletheon_abi::event::Event;

/// Extension trait for converting Events into Envelopes.
pub trait EventEnvelopeExt {
    /// Wrap this Event as an Envelope payload.
    /// The Event is serialized to JSON for cross-process compatibility.
    fn into_envelope(self, source: Endpoint, target: Target, pattern: Pattern) -> Envelope;
}

impl<E: Event> EventEnvelopeExt for E {
    fn into_envelope(self, source: Endpoint, target: Target, pattern: Pattern) -> Envelope {
        let priority = self.priority();
        let json = self.to_json();
        Envelope::new(source, target, pattern, Payload::Json(json)).with_priority(priority)
    }
}

/// Create a request envelope with JSON payload.
pub fn json_request(
    source: Endpoint,
    target: Target,
    value: serde_json::Value,
    timeout: std::time::Duration,
) -> Envelope {
    Envelope::request(source, target, Payload::Json(value), timeout)
}

/// Create a response envelope with JSON payload.
pub fn json_response(request: &Envelope, value: serde_json::Value) -> Envelope {
    Envelope::response(request, Payload::Json(value))
}

/// Create a topic publish envelope with JSON payload.
pub fn json_publish(source: Endpoint, topic: &str, value: serde_json::Value) -> Envelope {
    Envelope::publish(source, topic, Payload::Json(value))
}
