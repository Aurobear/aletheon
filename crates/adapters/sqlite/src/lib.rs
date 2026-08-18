//! Physical SQLite adapters.
//!
//! Ports remain owned by Runtime/Application; this crate owns only persistence
//! implementations and their versioned migration bundles.

pub mod approval;
pub mod approval_repository;
pub mod artifact;
pub mod capability_benchmark;
pub mod channel;
pub mod channel_projection;
pub mod checkpoint;
pub mod episode;
pub mod evaluation;
pub mod event_projection;
pub mod event_spine;
pub mod evolution;
pub mod exec_idempotency;
pub mod goal;
pub mod projection_set;
pub mod prompt_queue;
pub mod runtime_agent;
pub mod schema_migrations;
pub mod session;
pub mod settlement;
pub mod transaction_settlement;

pub use capability_benchmark::SqliteCapabilityRollupProjectionSink;
pub use channel::{ChannelStore, InsertOutcome};
pub use runtime_agent::SqliteAgentRunProjection;
pub use settlement::SqliteSettlementReceiptStore;
