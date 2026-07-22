use std::collections::HashMap;
use std::sync::Arc;

use fabric::{AgentError, RegistrationId, Registry};

use super::Tool;

/// Central registry for all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    proposal_confidences: HashMap<String, f32>,
    id_map: HashMap<RegistrationId, String>,
    next_id: u64,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            proposal_confidences: HashMap::new(),
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

    /// Declare trusted, host-only proposal confidence for a registered tool.
    ///
    /// This metadata is deliberately absent from `ToolDefinition` and tool
    /// input, so an LLM cannot author or override it.
    pub fn set_proposal_confidence(
        &mut self,
        name: &str,
        confidence: f32,
    ) -> Result<(), AgentError> {
        if !self.tools.contains_key(name) {
            return Err(AgentError::not_found(name));
        }
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(AgentError::config_missing(&format!(
                "proposal confidence for {name} must be finite and within [0,1]"
            )));
        }
        self.proposal_confidences
            .insert(name.to_owned(), confidence);
        Ok(())
    }

    /// Snapshot host-only proposal confidence metadata for planning.
    pub fn proposal_confidences(&self) -> HashMap<String, f32> {
        self.proposal_confidences.clone()
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

    pub fn register_robot_tools(
        &mut self,
        port: Arc<dyn fabric::types::embodiment::EmbodimentExecutionPort>,
    ) -> Result<Vec<RegistrationId>, AgentError> {
        use super::robot::{
            RobotCancelTool, RobotExecuteSkillTool, RobotGetStateTool, RobotListSkillsTool,
            RobotObserveTool, RobotSafeStopTool,
        };
        let registrations = [
            Arc::new(RobotObserveTool::new(port.clone())) as Arc<dyn Tool>,
            Arc::new(RobotGetStateTool::new(port.clone())),
            Arc::new(RobotListSkillsTool::new(port.clone())),
            Arc::new(RobotExecuteSkillTool::new(port.clone())),
            Arc::new(RobotCancelTool::new(port.clone())),
            Arc::new(RobotSafeStopTool::new(port)),
        ]
        .into_iter()
        .map(|tool| self.register(tool))
        .collect::<Result<Vec<_>, _>>()?;
        for name in [
            "robot.observe",
            "robot.get_state",
            "robot.list_skills",
            "robot.execute_skill",
            "robot.cancel",
            "robot.safe_stop",
        ] {
            self.set_proposal_confidence(name, 0.5)?;
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
            .ok_or_else(|| AgentError::not_found(&format!("{id:?}")))?;
        self.tools
            .remove(&name)
            .inspect(|_| {
                self.proposal_confidences.remove(&name);
            })
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
        Self::with_network_policy(fabric::network_policy::NetworkPolicy::default())
    }
}

impl ToolRegistry {
    /// Construct the built-in registry with daemon-trusted network authority.
    /// The policy is host configuration, never tool/model input.
    pub fn with_network_policy(policy: fabric::network_policy::NetworkPolicy) -> Self {
        Self::with_network_policy_and_search(policy, None)
    }

    pub fn with_network_policy_and_search(
        policy: fabric::network_policy::NetworkPolicy,
        search: Option<super::web_search::WebSearchConfig>,
    ) -> Self {
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
            .register(Arc::new(
                super::web_fetch::WebFetchTool::new().with_network_policy(policy.clone()),
            ))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(
                super::web_search::WebSearchTool::new()
                    .with_network_policy(policy)
                    .with_config(search),
            ))
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
        // Built-ins explicitly share a conservative host-authored baseline.
        // Deployments may replace individual values after registration.
        for name in registry
            .list()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            registry
                .set_proposal_confidence(&name, 0.5)
                .expect("built-in tool must be registered before metadata");
        }
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
                "expected tool '{name}' not found in registry"
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

    #[test]
    fn proposal_confidence_is_host_only_and_missing_by_default() {
        let mut reg = ToolRegistry::new();
        let tool = Arc::new(MockTool::new("ranked_tool"));
        Registry::<Arc<dyn Tool>>::register(&mut reg, tool).unwrap();

        assert!(!reg.proposal_confidences().contains_key("ranked_tool"));
        reg.set_proposal_confidence("ranked_tool", 0.75).unwrap();
        assert_eq!(reg.proposal_confidences()["ranked_tool"], 0.75);

        let definition = reg
            .definitions()
            .into_iter()
            .find(|definition| definition.name == "ranked_tool")
            .unwrap();
        let serialized = serde_json::to_value(definition).unwrap();
        assert!(serialized.get("proposal_confidence").is_none());
    }

    #[test]
    fn proposal_confidence_accepts_boundaries_and_rejects_invalid_values() {
        let mut reg = ToolRegistry::new();
        Registry::<Arc<dyn Tool>>::register(&mut reg, Arc::new(MockTool::new("bounded"))).unwrap();
        assert!(reg.set_proposal_confidence("bounded", 0.0).is_ok());
        assert!(reg.set_proposal_confidence("bounded", 1.0).is_ok());
        for invalid in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            assert!(reg.set_proposal_confidence("bounded", invalid).is_err());
        }
        assert_eq!(reg.proposal_confidences()["bounded"], 1.0);
    }
}
