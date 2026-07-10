//! Communication primitives — typed wrappers over the wire `Envelope`.
//!
//! Command / Query / Event / Stream make the *intent* of a message explicit at
//! the type level; each lowers to an `Envelope` with the correct `Pattern`.
//! `Mailbox` abstracts send/recv over the existing CommunicationBus.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::ipc::envelope::{Endpoint, Envelope, Pattern, Payload, Target};

/// Re-export the wire envelope as a primitive.
pub use crate::ipc::envelope::Envelope as WireEnvelope;

/// A command — perform an action, no response awaited.
pub struct Command {
    pub target: Target,
    pub payload: Payload,
}

impl Command {
    pub fn new(target: Target, payload: Payload) -> Self {
        Self { target, payload }
    }
    /// Lower to an `Envelope` (FireAndForget pattern).
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(source, self.target, Pattern::FireAndForget, self.payload)
    }
}

/// A query — request expecting a response within `timeout`.
pub struct Query {
    pub target: Target,
    pub payload: Payload,
    pub timeout: Duration,
}

impl Query {
    pub fn new(target: Target, payload: Payload, timeout: Duration) -> Self {
        Self {
            target,
            payload,
            timeout,
        }
    }
    /// Lower to a Request `Envelope`.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::request(source, self.target, self.payload, self.timeout)
    }
}

/// An event — async broadcast to a topic.
pub struct Event {
    pub topic: String,
    pub payload: Payload,
}

impl Event {
    pub fn new(topic: impl Into<String>, payload: Payload) -> Self {
        Self {
            topic: topic.into(),
            payload,
        }
    }
    /// Lower to a Publish `Envelope` targeting the topic.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(
            source,
            Target::Topic(self.topic),
            Pattern::Publish,
            self.payload,
        )
    }
}

/// A stream — continuous data flow keyed by a session id.
pub struct Stream {
    pub target: Target,
    pub session_id: u64,
    pub payload: Payload,
}

impl Stream {
    pub fn new(target: Target, session_id: u64, payload: Payload) -> Self {
        Self {
            target,
            session_id,
            payload,
        }
    }
    /// Lower to a Stream `Envelope`.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(
            source,
            self.target,
            Pattern::Stream {
                session_id: self.session_id,
            },
            self.payload,
        )
    }
}

/// A mailbox — abstract send/recv endpoint. Backed by CommunicationBus.
#[async_trait]
pub trait Mailbox: Send + Sync {
    async fn send(&self, envelope: Envelope) -> Result<()>;
    async fn recv(&self) -> Option<Envelope>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lowers_to_fire_and_forget() {
        let cmd = Command::new(Target::Broadcast, Payload::Empty);
        let env = cmd.into_envelope(Endpoint::System);
        assert!(matches!(env.pattern, Pattern::FireAndForget));
    }

    #[test]
    fn query_lowers_to_request() {
        let q = Query::new(
            Target::Broadcast,
            Payload::Empty,
            Duration::from_millis(500),
        );
        let env = q.into_envelope(Endpoint::System);
        assert!(matches!(env.pattern, Pattern::Request { .. }));
    }

    #[test]
    fn event_targets_topic() {
        let e = Event::new("evolution", Payload::Empty);
        let env = e.into_envelope(Endpoint::System);
        assert!(matches!(env.target, Target::Topic(ref t) if t == "evolution"));
    }
}
