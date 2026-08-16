//! Tool registry and execution.

pub mod agent_control;
pub mod agent_tool;
pub mod agora_task_tools;
pub mod apply_patch;
pub mod artifact_read;
pub mod bash_exec;
pub mod change_transaction;
pub mod code_graph;
pub mod ebpf_compile;
pub mod executor;
pub mod exposure;
pub mod file_read;
pub mod file_search;
pub mod file_write;
pub mod git_tools;
pub mod glob;
pub mod grep;
pub mod kernel_build;
pub mod managed_command;
pub mod module_build;
pub mod module_load;
pub(crate) mod mutation_path;
pub mod output;
pub(crate) mod overview_guard;
pub mod process_list;
pub mod registry;
mod registry_error;
pub mod repo_inspect;
pub(crate) mod repository;
pub mod robot;
mod scoped_filesystem;
pub mod script_tool;
pub mod search;
pub mod skill_tools;
pub mod structured_patch;
pub mod system_status;
pub mod task_tools;
pub mod toolchain_status;
pub mod toolset;
pub mod web_fetch;
pub mod web_search;
pub(crate) mod workspace_version;

// Re-export types from fabric (the canonical definitions)
pub use ::contracts::tool::{ConcurrencyClass, ToolExposure};
pub use ::contracts::tool::{
    PermissionLevel, Tool, ToolContext, ToolExecutionDescriptor, ToolResult, ToolResultMeta,
};
pub use registry::{RegistrationId, Registry, ToolRegistry};
pub use toolset::ToolsetRegistry;
