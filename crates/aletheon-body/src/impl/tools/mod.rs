//! Tool registry and execution.

#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
pub mod acix_tools;
pub mod apply_patch;
pub mod bash_exec;
pub mod code_graph;
pub mod ebpf_compile;
pub mod executor;
pub mod exposure;
pub mod file_read;
pub mod file_search;
pub mod glob;
pub mod file_write;
pub mod kernel_build;
pub mod module_build;
pub mod module_load;
pub mod output;
pub mod process_list;
pub mod registry;
pub mod script_tool;
pub mod search;
pub mod system_status;
pub mod task_tools;
pub mod toolset;

// Re-export types from aletheon-abi (the canonical definitions)
pub use aletheon_abi::tool::{ConcurrencyClass, ToolExposure};
pub use aletheon_abi::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
pub use registry::ToolRegistry;
pub use toolset::ToolsetRegistry;
