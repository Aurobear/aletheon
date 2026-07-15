//! Optional GBrain supplemental-memory transport.

pub mod mcp_adapter;
pub mod worker;

pub use mcp_adapter::{
    GbrainAdapterError, GbrainErrorCategory, GbrainHealth, GbrainHealthState, GbrainMcpAdapter,
    GbrainSchemaStatus, GbrainSearchHit,
};

pub use worker::{DrainReport, GbrainWorker};
