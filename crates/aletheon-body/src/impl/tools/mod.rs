//! Tool registry and execution for Argos (migrated from argos-tools).

pub mod registry;
pub mod bash_exec;
pub mod file_read;
pub mod file_write;
pub mod system_status;
pub mod process_list;
pub mod output;
pub mod ebpf_compile;
pub mod module_build;
pub mod module_load;
pub mod kernel_build;
pub mod executor;
pub mod search;
pub mod toolset;
#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
pub mod acix_tools;
pub mod exposure;

// Re-export types from aletheon-abi (the canonical definitions)
pub use aletheon_abi::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
pub use aletheon_abi::tool::{ToolExposure, ConcurrencyClass};
pub use registry::ToolRegistry;
pub use toolset::ToolsetRegistry;
