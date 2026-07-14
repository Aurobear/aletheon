use std::collections::HashMap;
use std::sync::Arc;

use fabric::{AgentError, RegistrationId, Registry};

use super::Tool;

/// Central registry for all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    id_map: HashMap<RegistrationId, String>,
    next_id: u64,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            id_map: HashMap::new(),
            next_id: 1,
        }
    }

    /// Get a tool by name (inherent method, shadows trait method for direct calls).
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool definitions for LLM (name, description, schema).
    pub fn definitions(&self) -> Vec<fabric::ToolDefinition> {
        self.tools
            .values()
            .map(|t| fabric::ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub fn register_google_read_tools(
        &mut self,
        gmail: Option<Arc<dyn crate::tools::google::GmailCapability>>,
        calendar: Option<Arc<dyn crate::tools::google::CalendarCapability>>,
        accounts: Arc<dyn crate::tools::google::GoogleAccountResolver>,
    ) -> Result<Vec<RegistrationId>, AgentError> {
        let mut registrations = Vec::new();
        if let Some(gmail) = gmail {
            registrations.push(self.register(Arc::new(
                crate::tools::google::GoogleGmailSearchTool::new(gmail.clone(), accounts.clone()),
            ))?);
            registrations.push(self.register(Arc::new(
                crate::tools::google::GoogleGmailReadTool::new(gmail, accounts.clone()),
            ))?);
        }
        if let Some(calendar) = calendar {
            registrations.push(self.register(Arc::new(
                crate::tools::google::GoogleCalendarListTool::new(calendar, accounts),
            ))?);
        }
        Ok(registrations)
    }
}

impl Registry<Arc<dyn Tool>> for ToolRegistry {
    fn register(&mut self, tool: Arc<dyn Tool>) -> Result<RegistrationId, AgentError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(AgentError::already_exists(&name));
        }
        let id = RegistrationId(self.next_id);
        self.next_id += 1;
        self.id_map.insert(id, name.clone());
        self.tools.insert(name, tool);
        Ok(id)
    }

    fn unregister(&mut self, id: RegistrationId) -> Result<Arc<dyn Tool>, AgentError> {
        let name = self
            .id_map
            .remove(&id)
            .ok_or_else(|| AgentError::not_found(&format!("{:?}", id)))?;
        self.tools
            .remove(&name)
            .ok_or_else(|| AgentError::not_found(&name))
    }

    fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    fn len(&self) -> usize {
        self.tools.len()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        // Register built-in tools — panics on duplicate names (should never happen)
        registry
            .register(Arc::new(super::bash_exec::BashExecTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::file_read::FileReadTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::file_write::FileWriteTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::system_status::SystemStatusTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::process_list::ProcessListTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::ebpf_compile::EbpfCompileTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::module_build::ModuleBuildTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::module_load::ModuleLoadTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::kernel_build::KernelBuildTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::code_graph::CodeGraphTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::file_search::FileSearchTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::apply_patch::ApplyPatchTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::glob::GlobTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::grep::GrepTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::web_fetch::WebFetchTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::web_search::WebSearchTool))
            .expect("duplicate built-in tool");
        // Task tools share a single TaskStore.
        let task_store = super::task_tools::new_shared_task_store();
        registry
            .register(Arc::new(super::task_tools::TaskCreateTool::new(
                task_store.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::task_tools::TaskUpdateTool::new(
                task_store.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::task_tools::TaskListTool::new(
                task_store.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::task_tools::TaskGetTool::new(
                task_store.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::Registry;

    /// A minimal mock tool for testing.
    struct MockTool {
        tool_name: String,
    }

    impl MockTool {
        fn new(name: &str) -> Self {
            Self {
                tool_name: name.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.tool_name
        }

        fn description(&self) -> &str {
            "mock tool for testing"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn permission_level(&self) -> fabric::tool::PermissionLevel {
            fabric::tool::PermissionLevel::L0
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &fabric::tool::ToolContext,
        ) -> fabric::tool::ToolResult {
            fabric::tool::ToolResult {
                content: String::new(),
                is_error: false,
                metadata: fabric::tool::ToolResultMeta::default(),
            }
        }

        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(MockTool {
                tool_name: self.tool_name.clone(),
            })
        }
    }

    #[test]
    fn register_and_unregister() {
        let mut reg = ToolRegistry::new();
        let tool = Arc::new(MockTool::new("my_tool"));

        let id = Registry::<Arc<dyn Tool>>::register(&mut reg, tool).unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("my_tool"));
        assert_eq!(reg.names(), vec!["my_tool"]);

        let removed = reg.unregister(id).unwrap();
        assert_eq!(removed.name(), "my_tool");
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains("my_tool"));
    }

    #[test]
    fn default_registry_contains_expected_tools() {
        let reg = ToolRegistry::default();
        let names: Vec<&str> = reg.names();
        let expected = [
            "glob",
            "grep",
            "web_fetch",
            "web_search",
            "task_create",
            "task_update",
            "task_list",
            "task_get",
        ];
        for name in expected {
            assert!(
                names.contains(&name),
                "expected tool '{}' not found in registry",
                name
            );
        }
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = ToolRegistry::new();
        let tool1 = Arc::new(MockTool::new("dup_tool"));
        let tool2 = Arc::new(MockTool::new("dup_tool"));

        let _ = Registry::<Arc<dyn Tool>>::register(&mut reg, tool1).unwrap();
        let result = Registry::<Arc<dyn Tool>>::register(&mut reg, tool2);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("dup_tool"));
        assert!(err.message.contains("already registered"));
    }
}
