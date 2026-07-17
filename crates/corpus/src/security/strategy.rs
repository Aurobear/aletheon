use fabric::tool::PermissionLevel;

/// Defines how the runner should execute a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolExecutionStrategy {
    /// Execute in-process with workspace confinement only (read-only tools, no sandbox).
    InProcess,
    /// Execute through the sandbox backend (bash_exec, file_write, apply_patch,
    /// ebpf_compile, kernel_build, module_build, module_load, script_tool).
    Sandboxed,
    /// Execute via the network proxy (web_fetch, web_search). Phase 2+.
    NetworkProxied { allowed_domains: Vec<String> },
    /// Tool requires exec-server isolation (Phase 2+). In Phase 1, treated as Sandboxed.
    ExecServerRequired,
}

/// Resolve the execution strategy for a given tool name and permission level.
/// Maps to the tool classification table from tool-execution-hardening-plan §3.1.2.
pub fn resolve_strategy(
    tool_name: &str,
    _permission_level: PermissionLevel,
) -> ToolExecutionStrategy {
    match tool_name {
        // Shell/script tools: always sandboxed
        "bash_exec" | "script_tool" => ToolExecutionStrategy::Sandboxed,

        // File-mutating tools: sandboxed
        "file_write" | "apply_patch" => ToolExecutionStrategy::Sandboxed,

        // Build tools: sandboxed with longer timeouts (timeout handled at execution site)
        "ebpf_compile" | "kernel_build" | "module_build" | "module_load" => {
            ToolExecutionStrategy::Sandboxed
        }

        // Network tools: network-proxied (Phase 2)
        "web_fetch" | "web_search" => ToolExecutionStrategy::NetworkProxied {
            allowed_domains: vec![],
        },

        // Read-only tools: in-process is fine
        _ => ToolExecutionStrategy::InProcess,
    }
}
