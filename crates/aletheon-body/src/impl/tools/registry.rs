use std::collections::HashMap;
use std::sync::Arc;

use super::Tool;

/// Central registry for all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool definitions for LLM (name, description, schema).
    pub fn definitions(&self) -> Vec<aletheon_abi::ToolDefinition> {
        self.tools
            .values()
            .map(|t| aletheon_abi::ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        // Register built-in tools
        registry.register(Arc::new(super::bash_exec::BashExecTool));
        registry.register(Arc::new(super::file_read::FileReadTool));
        registry.register(Arc::new(super::file_write::FileWriteTool));
        registry.register(Arc::new(super::system_status::SystemStatusTool));
        registry.register(Arc::new(super::process_list::ProcessListTool));
        registry.register(Arc::new(super::ebpf_compile::EbpfCompileTool));
        registry.register(Arc::new(super::module_build::ModuleBuildTool));
        registry.register(Arc::new(super::module_load::ModuleLoadTool));
        registry.register(Arc::new(super::kernel_build::KernelBuildTool));
        registry.register(Arc::new(super::code_graph::CodeGraphTool));
        registry.register(Arc::new(super::file_search::FileSearchTool));
        registry.register(Arc::new(super::apply_patch::ApplyPatchTool));
        registry
    }
}
