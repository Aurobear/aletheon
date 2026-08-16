//! Inter-process communication — protocol, transport, and bus.

pub mod envelope_v2;
pub mod stream;

// Turn event stream types

pub use stream::{
    SchemaRejection, TurnEventSender, TurnEventStream, TurnEventV1, TURN_EVENT_SCHEMA,
};
