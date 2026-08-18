//! Session persistence adapters (moved from `executive::adapters::session`).

pub mod canonical_store;
pub mod event_sourced_store;
pub mod protocol_event_store;
pub mod store;
pub mod turn_event_journal;
pub mod turn_history;
pub mod turn_identity;
pub mod turn_session_port;

pub use canonical_store::CanonicalSessionStore;
pub use event_sourced_store::EventSourcedSessionStore;
pub use store::SessionStore;

pub use protocol_event_store::SqliteSessionProtocolEventStore;
pub use turn_identity::CanonicalTurnIdentityAdapter;
pub use turn_session_port::SessionAppendTurnPort;
