//! Optional GBrain supplemental-memory transport.

pub mod bootstrap;
pub mod mcp_adapter;
pub mod worker;

pub use mcp_adapter::{
    GbrainAdapterError, GbrainErrorCategory, GbrainHealth, GbrainHealthState, GbrainMcpAdapter,
    GbrainSchemaStatus, GbrainSearchHit,
};

pub use worker::{DrainReport, GbrainWorker};

pub use bootstrap::{
    backend_config, build_supplemental_memory_runtime,
    build_supplemental_memory_runtime_with_retention, GbrainMemoryRuntime,
};
