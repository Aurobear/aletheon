//! Optional Supplemental supplemental-memory transport.

pub mod bootstrap;
pub mod mcp_adapter;

pub use mcp_adapter::{
    McpSupplementalBindingNegotiator, SupplementalAdapterError, SupplementalAdapterErrorCategory,
    SupplementalHealth, SupplementalHealthState, SupplementalMcpAdapter, SupplementalSchemaStatus,
    SupplementalSearchHit,
};

pub use mnemosyne::supplemental::{DrainReport, SupplementalDeliveryWorker};

pub use bootstrap::{
    backend_config, build_supplemental_memory_runtime,
    build_supplemental_memory_runtime_with_retention, SupplementalMemoryRuntime,
};
