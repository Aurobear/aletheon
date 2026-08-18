//! GBrain supplemental-memory transport and governed context-recall adapters.
//!
//! Owns the MCP supplemental binding/negotiation, the supplemental-memory
//! runtime bootstrap, and the synchronous MemoryGateway recall adapter. The
//! `aletheon` host only composes these adapters into bootstrap; credential,
//! marker SHA and source binding stay in adapter-private types.

pub mod bootstrap;
pub mod context_memory;
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

pub use context_memory::MemoryGatewayContextRecall;
