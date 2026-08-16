//! Session persistence adapters (moved from `executive::adapters::session`).

pub mod canonical_store;
pub mod event_sourced_store;
pub mod store;

pub use canonical_store::CanonicalSessionStore;
pub use event_sourced_store::EventSourcedSessionStore;
pub use store::SessionStore;
