// crates/fabric/src/envelope.rs

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Unique message identifier.
pub type EnvelopeId = u64;

/// Module identifiers for intra-process routing.
///
/// Names track the 7-subsystem model / crate names (RFC-018 D3). `Perception`
/// is retained as a routing endpoint for perception events (it is not a
/// top-level crate). Not persisted to disk; purely in-process/same-build wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleId {
    Cognit,
    Dasein,
    Mnemosyne,
    Corpus,
    Metacog,
    Executive,
    Perception,
}

/// Sender identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Endpoint {
    /// Internal module.
    Module(ModuleId),
    /// Agent process (Pid from ipc.rs).
    Agent(u64),
    /// System-level (kernel).
    System,
}

/// Receiver target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// Point-to-point: specific module.
    Module(ModuleId),
    /// Point-to-point: specific Agent.
    Agent(u64),
    /// Topic subscription: broadcast to all subscribers.
    Topic(String),
    /// Global broadcast.
    Broadcast,
}

/// Communication pattern — determines wait semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    /// Synchronous wait for response.
    Request { timeout_ms: u64 },
    /// Reply to a Request.
    Response,
    /// Async broadcast, no wait.
    Publish,
    /// Async, don't care about delivery.
    FireAndForget,
    /// Continuous data stream.
    Stream { session_id: u64 },
}

/// Payload — unified data format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    /// Structured JSON data (default).
    Json(serde_json::Value),
    /// Binary data.
    Binary(Vec<u8>),
    /// No payload.
    Empty,
}

/// Unified message envelope — wire format for all communication.
/// Analogous to Linux sk_buff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Unique message ID.
    pub id: EnvelopeId,
    /// Correlation ID for request-response pairing.
    pub correlation_id: Option<EnvelopeId>,
    /// Sender.
    pub source: Endpoint,
    /// Receiver.
    pub target: Target,
    /// Communication pattern.
    pub pattern: Pattern,
    /// Priority (reuses existing Priority from event.rs).
    pub priority: crate::events::types::Priority,
    /// Message time-to-live in milliseconds. None = no expiry.
    pub ttl_ms: Option<u64>,
    /// Actual data.
    pub payload: Payload,
    /// Creation timestamp (millis since epoch).
    pub timestamp_ms: u64,
}

static ENVELOPE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Envelope {
    /// Create a new Envelope with auto-generated ID and timestamp.
    pub fn new(source: Endpoint, target: Target, pattern: Pattern, payload: Payload) -> Self {
        Self {
            id: ENVELOPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            correlation_id: None,
            source,
            target,
            pattern,
            priority: crate::events::types::Priority::Normal,
            ttl_ms: None,
            payload,
            timestamp_ms: millis_now(),
        }
    }

    /// Create a Request envelope.
    pub fn request(source: Endpoint, target: Target, payload: Payload, timeout: Duration) -> Self {
        Self::new(
            source,
            target,
            Pattern::Request {
                timeout_ms: timeout.as_millis() as u64,
            },
            payload,
        )
    }

    /// Create a Response envelope correlated to a request.
    pub fn response(request: &Envelope, payload: Payload) -> Self {
        Self {
            id: ENVELOPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            correlation_id: Some(request.id),
            source: request.target.clone().into_endpoint(),
            target: request.source.clone().into_target(),
            pattern: Pattern::Response,
            priority: request.priority,
            ttl_ms: None,
            payload,
            timestamp_ms: millis_now(),
        }
    }

    /// Create a Publish envelope for topic broadcast.
    pub fn publish(source: Endpoint, topic: &str, payload: Payload) -> Self {
        Self::new(
            source,
            Target::Topic(topic.to_string()),
            Pattern::Publish,
            payload,
        )
    }

    /// Create a FireAndForget envelope.
    pub fn fire_and_forget(source: Endpoint, target: Target, payload: Payload) -> Self {
        Self::new(source, target, Pattern::FireAndForget, payload)
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: crate::events::types::Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Set TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl_ms = Some(ttl.as_millis() as u64);
        self
    }

    /// Check if this envelope has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl_ms) = self.ttl_ms {
            millis_now() > self.timestamp_ms + ttl_ms
        } else {
            false
        }
    }
}

impl Target {
    /// Convert Target to Endpoint (for response routing).
    pub fn into_endpoint(self) -> Endpoint {
        match self {
            Target::Module(m) => Endpoint::Module(m),
            Target::Agent(p) => Endpoint::Agent(p),
            Target::Topic(_) | Target::Broadcast => Endpoint::System,
        }
    }
}

impl Endpoint {
    /// Convert Endpoint to Target.
    pub fn into_target(self) -> Target {
        match self {
            Endpoint::Module(m) => Target::Module(m),
            Endpoint::Agent(p) => Target::Agent(p),
            Endpoint::System => Target::Broadcast,
        }
    }
}

fn millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Extension trait for converting Events into Envelopes.
pub trait EventEnvelopeExt {
    /// Wrap this Event as an Envelope payload.
    /// The Event is serialized to JSON for cross-process compatibility.
    fn into_envelope(self, source: Endpoint, target: Target, pattern: Pattern) -> Envelope;
}

impl<E: crate::events::types::Event> EventEnvelopeExt for E {
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
