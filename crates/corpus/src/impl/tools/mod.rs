//! Tool registry and execution.

#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
pub mod acix_tools;
pub mod agent_tool;
pub mod apply_patch;
pub mod bash_exec;
pub mod code_graph;
pub mod ebpf_compile;
pub mod executor;
pub mod exposure;
pub mod file_read;
pub mod file_search;
pub mod file_write;
pub mod glob;
pub mod grep;
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
pub mod web_fetch;
pub mod web_search;

// Re-export types from aletheon-abi (the canonical definitions)
pub use base::tool::{ConcurrencyClass, ToolExposure};
pub use base::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
pub use registry::ToolRegistry;
pub use toolset::ToolsetRegistry;
