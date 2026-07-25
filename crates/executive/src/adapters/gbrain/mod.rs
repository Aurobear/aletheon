//! Optional Supplemental supplemental-memory transport.

pub mod bootstrap;
pub mod mcp_adapter;
pub mod worker;

pub use mcp_adapter::{
    SupplementalAdapterError, SupplementalAdapterErrorCategory, SupplementalHealth,
    SupplementalHealthState, SupplementalMcpAdapter, SupplementalSchemaStatus,
    SupplementalSearchHit,
};

pub use worker::{DrainReport, SupplementalDeliveryWorker};

pub use bootstrap::{
    backend_config, build_supplemental_memory_runtime,
    build_supplemental_memory_runtime_with_retention, SupplementalMemoryRuntime,
};
