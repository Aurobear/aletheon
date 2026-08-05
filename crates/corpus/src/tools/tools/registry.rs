use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use fabric::{tool::ToolExposure, AgentError, RegistrationId, Registry};

use super::search::{tool_search::ToolSearchTool, BM25Catalog, CatalogEntry};
use super::Tool;

/// Central registry for all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    proposal_confidences: HashMap<String, f32>,
    id_map: HashMap<RegistrationId, String>,
    package_tool_owners: HashMap<String, String>,
    next_id: u64,
    search_catalog: Option<Arc<RwLock<BM25Catalog>>>,
    change_transactions: Option<super::change_transaction::ChangeTransactionRegistry>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            proposal_confidences: HashMap::new(),
            id_map: HashMap::new(),
            package_tool_owners: HashMap::new(),
            next_id: 1,
            search_catalog: None,
            change_transactions: None,
        }
    }

    /// Get a tool by name (inherent method, shadows trait method for direct calls).
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Return the Host-shared change transaction authority used by built-in
    /// mutation and managed-validation tools. Extension-only registries do not
    /// own this authority.
    pub fn change_transactions(
        &self,
    ) -> Option<super::change_transaction::ChangeTransactionRegistry> {
        self.change_transactions.clone()
    }

    /// Atomically replace every tool owned by one extension package.
    pub fn replace_package_tools(
        &mut self,
        owner: &str,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<(), AgentError> {
        self.replace_package_tool_sets(&[owner.to_owned()], vec![(owner.to_owned(), tools)])
    }

    /// Atomically replace a set of package owners while preserving built-ins,
    /// legacy tools, and package owners outside the supplied replacement set.
    pub fn replace_package_tool_sets(
        &mut self,
        replaced_owners: &[String],
        replacements: Vec<(String, Vec<Arc<dyn Tool>>)>,
    ) -> Result<(), AgentError> {
        self.validate_package_tool_sets(replaced_owners, &replacements)?;
        let replaced: std::collections::HashSet<_> = replaced_owners.iter().cloned().collect();
        let mut incoming = HashMap::<String, (String, Arc<dyn Tool>)>::new();
        for (owner, tools) in replacements {
            for tool in tools {
                let name = tool.name().to_owned();
                incoming.insert(name, (owner.clone(), tool));
            }
        }

        let removed_names: Vec<_> = self
            .package_tool_owners
            .iter()
            .filter(|(_, owner)| replaced.contains(*owner))
            .map(|(name, _)| name.clone())
            .collect();
        for name in removed_names {
            self.tools.remove(&name);
            self.proposal_confidences.remove(&name);
            self.package_tool_owners.remove(&name);
            self.id_map
                .retain(|_, registered_name| registered_name != &name);
        }

        for (name, (owner, tool)) in incoming {
            let id = RegistrationId(self.next_id);
            self.next_id += 1;
            self.id_map.insert(id, name.clone());
            self.package_tool_owners.insert(name.clone(), owner);
            self.tools.insert(name.clone(), tool);
            // Package tools are admitted by the host's extension validator, so
            // they receive the same conservative host-authored planning baseline
            // as built-ins. This metadata is never supplied by the package.
            self.proposal_confidences.insert(name, 0.5);
        }
        self.refresh_search_catalog();
        Ok(())
    }

    pub fn validate_package_tool_sets(
        &self,
        replaced_owners: &[String],
        replacements: &[(String, Vec<Arc<dyn Tool>>)],
    ) -> Result<(), AgentError> {
        let replaced: std::collections::HashSet<_> = replaced_owners.iter().cloned().collect();
        if replaced.iter().any(|owner| owner.trim().is_empty()) {
            return Err(AgentError::config_missing(
                "package tool owner cannot be empty",
            ));
        }
        let mut incoming = std::collections::HashSet::new();
        for (owner, tools) in replacements {
            if !replaced.contains(owner) {
                return Err(AgentError::config_missing(&format!(
                    "package tool replacement owner '{owner}' is outside its replacement set"
                )));
            }
            for tool in tools {
                let name = tool.name();
                if name.trim().is_empty() {
                    return Err(AgentError::config_missing(
                        "package tool name cannot be empty",
                    ));
                }
                if !incoming.insert(name.to_owned()) {
                    return Err(AgentError::already_exists(name));
                }
                if self.tools.contains_key(name)
                    && self
                        .package_tool_owners
                        .get(name)
                        .is_none_or(|registered_owner| !replaced.contains(registered_owner))
                {
                    return Err(AgentError::already_exists(name));
                }
            }
        }
        Ok(())
    }

    /// Compile the definitions that would be visible after an atomic package
    /// replacement, without mutating the live registry. Profiles are validated
    /// against this candidate catalog so removed capabilities cannot linger.
    pub fn candidate_package_definitions(
        &self,
        replaced_owners: &[String],
        replacements: &[(String, Vec<Arc<dyn Tool>>)],
    ) -> Result<(Vec<fabric::ToolDefinition>, Vec<fabric::ToolDefinition>), AgentError> {
        self.validate_package_tool_sets(replaced_owners, replacements)?;
        let replaced = replaced_owners
            .iter()
            .collect::<std::collections::HashSet<_>>();
        let retained = self.tools.iter().filter(|(name, _)| {
            self.package_tool_owners
                .get(*name)
                .is_none_or(|owner| !replaced.contains(owner))
        });
        let incoming = replacements
            .iter()
            .flat_map(|(_, tools)| tools.iter().map(|tool| (tool.name(), tool.as_ref())));
        let all = retained
            .map(|(_, tool)| (tool.name(), tool.as_ref()))
            .chain(incoming)
            .collect::<Vec<_>>();
        let mut visible = all
            .iter()
            .filter(|(_, tool)| {
                matches!(
                    tool.exposure(),
                    ToolExposure::Direct | ToolExposure::DirectModelOnly
                )
            })
            .map(|(name, tool)| fabric::ToolDefinition {
                name: (*name).to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
            })
            .collect::<Vec<_>>();
        let mut authorized = all
            .iter()
            .filter(|(_, tool)| tool.exposure() != ToolExposure::Hidden)
            .map(|(name, tool)| fabric::ToolDefinition {
                name: (*name).to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
            })
            .collect::<Vec<_>>();
        visible.sort_by(|left, right| left.name.cmp(&right.name));
        authorized.sort_by(|left, right| left.name.cmp(&right.name));
        Ok((visible, authorized))
    }

    pub fn remove_package_tools(&mut self, owner: &str) -> Result<(), AgentError> {
        self.replace_package_tool_sets(&[owner.to_owned()], Vec::new())
    }

    /// Get tool definitions for LLM (name, description, schema).
    pub fn definitions(&self) -> Vec<fabric::ToolDefinition> {
        let mut definitions: Vec<_> = self
            .tools
            .values()
            .filter(|tool| {
                matches!(
                    tool.exposure(),
                    ToolExposure::Direct | ToolExposure::DirectModelOnly
                )
            })
            .map(|t| fabric::ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    /// Snapshot every executable tool for host-side authorization and profile
    /// validation. Unlike [`Self::definitions`], this includes deferred tools;
    /// callers must not pass this catalog wholesale to a model request.
    pub fn profile_definitions(&self) -> Vec<fabric::ToolDefinition> {
        let mut definitions: Vec<_> = self
            .tools
            .values()
            .filter(|tool| tool.exposure() != ToolExposure::Hidden)
            .map(|tool| fabric::ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    /// Bind the existing BM25 catalog to this registry and expose its single
    /// bridge tool. Later registrations refresh the same shared catalog.
    pub fn enable_tool_search(&mut self) -> Result<RegistrationId, AgentError> {
        if self.tools.contains_key("tool_search") {
            return Err(AgentError::already_exists("tool_search"));
        }
        let catalog = Arc::new(RwLock::new(self.build_search_catalog()));
        self.search_catalog = Some(catalog.clone());
        self.register(Arc::new(ToolSearchTool::new(catalog)))
    }

    fn build_search_catalog(&self) -> BM25Catalog {
        BM25Catalog::build(
            self.tools
                .values()
                .filter(|tool| tool.name() != "tool_search")
                .map(|tool| {
                    CatalogEntry::new(
                        tool.name(),
                        tool.description(),
                        &tool.search_text(),
                        tool.exposure(),
                    )
                })
                .collect(),
        )
    }

    fn refresh_search_catalog(&self) {
        if let Some(catalog) = &self.search_catalog {
            *catalog.write().expect("tool catalog lock poisoned") = self.build_search_catalog();
        }
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

    /// Replace the legacy standalone task store with the authoritative Agora
    /// task graph. Existing tool names stay stable for model profiles.
    pub fn bind_agora_task_tools(
        &mut self,
        service: Arc<dyn fabric::AgoraService>,
        host_process: fabric::ProcessId,
    ) -> Result<(), AgentError> {
        for name in [
            "task_create",
            "task_update",
            "task_list",
            "task_get",
            "request_user_input",
        ] {
            self.tools.remove(name);
            self.proposal_confidences.remove(name);
            self.id_map
                .retain(|_, registered_name| registered_name != name);
        }
        for tool in super::agora_task_tools::AgoraTaskTools::new(service, host_process).tools() {
            let name = tool.name().to_owned();
            self.register(tool)?;
            self.set_proposal_confidence(&name, 0.5)?;
        }
        self.refresh_search_catalog();
        Ok(())
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
            "robot_observe",
            "robot_get_state",
            "robot_list_skills",
            "robot_execute_skill",
            "robot_cancel",
            "robot_safe_stop",
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
        self.refresh_search_catalog();
        Ok(id)
    }

    fn unregister(&mut self, id: RegistrationId) -> Result<Arc<dyn Tool>, AgentError> {
        let name = self
            .id_map
            .remove(&id)
            .ok_or_else(|| AgentError::not_found(&format!("{id:?}")))?;
        let removed = self
            .tools
            .remove(&name)
            .inspect(|_| {
                self.proposal_confidences.remove(&name);
            })
            .ok_or_else(|| AgentError::not_found(&name))?;
        self.refresh_search_catalog();
        Ok(removed)
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
        Self::with_network_policy_search_and_tasks(policy, search, None)
    }

    /// Same as [`Self::with_network_policy_and_search`], but allows the task
    /// store to be backed by a SQLite database at `tasks_db` so tasks
    /// survive daemon restarts. `None` keeps the existing in-memory-only
    /// behavior.
    pub fn with_network_policy_search_and_tasks(
        policy: fabric::network_policy::NetworkPolicy,
        search: Option<super::web_search::WebSearchConfig>,
        tasks_db: Option<std::path::PathBuf>,
    ) -> Self {
        let mut registry = Self::new();
        let change_transactions = super::change_transaction::ChangeTransactionRegistry::default();
        registry.change_transactions = Some(change_transactions.clone());
        // Register built-in tools — panics on duplicate names (should never happen)
        registry
            .register(Arc::new(super::bash_exec::BashExecTool))
            .expect("duplicate built-in tool");
        let command_sessions =
            super::managed_command::ManagedCommandSessions::with_change_transactions(
                change_transactions.clone(),
            );
        registry
            .register(Arc::new(super::managed_command::ExecCommandTool::new(
                command_sessions.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::managed_command::WriteStdinTool::new(
                command_sessions.clone(),
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::managed_command::ValidationRunTool::new(
                command_sessions,
            )))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::file_read::FileReadTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(
                super::change_transaction::TransactionalRepoInspectTool::new(
                    change_transactions.clone(),
                ),
            ))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::artifact_read::ArtifactReadTool::default()))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(
                super::change_transaction::TransactionalFileWriteTool::new(
                    change_transactions.clone(),
                ),
            ))
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
            .register(Arc::new(
                super::change_transaction::TransactionalApplyPatchTool::new(
                    change_transactions.clone(),
                ),
            ))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::glob::GlobTool))
            .expect("duplicate built-in tool");
        registry
            .register(Arc::new(super::grep::GrepTool))
            .expect("duplicate built-in tool");
        // Git tools: read-only (status/diff/log/show) plus safe write/undo
        // (restore/stash/reset). These were defined but never registered, so
        // the agent previously had no git capability at all.
        for tool in super::git_tools::git_tools()
            .into_iter()
            .filter(|tool| tool.name() != "git_diff")
        {
            registry
                .register(tool)
                .expect("duplicate built-in git tool");
        }
        registry
            .register(Arc::new(
                super::change_transaction::TransactionalGitDiffTool::new(
                    change_transactions.clone(),
                ),
            ))
            .expect("duplicate built-in git tool");
        // Acceptance and rollback are Host review actions, not model tools.
        // The shared registry remains available to Executive through the
        // typed getter above; exposing either action here would create a
        // second settlement writer controlled by model output.
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
        // Task tools share a single TaskStore, optionally persisted to SQLite.
        let task_store = match tasks_db {
            Some(path) => super::task_tools::new_persistent_task_store(&path),
            None => super::task_tools::new_shared_task_store(),
        };
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
            .enable_tool_search()
            .expect("tool_search must be registered once");
        registry
            .set_proposal_confidence("tool_search", 0.5)
            .expect("tool_search metadata follows registration");
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
            "tool_search",
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
    fn default_registry_exposes_tool_search_to_the_model() {
        let reg = ToolRegistry::default();
        assert!(reg
            .definitions()
            .iter()
            .any(|definition| definition.name == "tool_search"));
        assert!(!reg
            .definitions()
            .iter()
            .any(|definition| definition.name == "artifact_read"));
        assert!(reg
            .profile_definitions()
            .iter()
            .any(|definition| definition.name == "artifact_read"));
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
    fn package_tool_replacement_is_owner_scoped_and_protects_builtins() {
        let mut reg = ToolRegistry::new();
        Registry::<Arc<dyn Tool>>::register(&mut reg, Arc::new(MockTool::new("builtin"))).unwrap();
        reg.replace_package_tools("pkg.one", vec![Arc::new(MockTool::new("pkg_tool"))])
            .unwrap();
        assert!(reg.contains("builtin"));
        assert!(reg.contains("pkg_tool"));
        assert_eq!(reg.proposal_confidences()["pkg_tool"], 0.5);

        reg.replace_package_tools("pkg.one", vec![Arc::new(MockTool::new("replacement"))])
            .unwrap();
        assert!(reg.contains("builtin"));
        assert!(!reg.contains("pkg_tool"));
        assert!(reg.contains("replacement"));
        assert!(!reg.proposal_confidences().contains_key("pkg_tool"));
        assert_eq!(reg.proposal_confidences()["replacement"], 0.5);

        let collision =
            reg.replace_package_tools("pkg.one", vec![Arc::new(MockTool::new("builtin"))]);
        assert!(collision.is_err());
        assert!(reg.contains("replacement"));
        reg.remove_package_tools("pkg.one").unwrap();
        assert!(reg.contains("builtin"));
        assert!(!reg.contains("replacement"));
    }

    #[test]
    fn definition_snapshots_are_sorted_by_name() {
        let mut reg = ToolRegistry::new();
        for name in ["zeta", "alpha", "middle"] {
            Registry::<Arc<dyn Tool>>::register(&mut reg, Arc::new(MockTool::new(name))).unwrap();
        }

        let names = |definitions: Vec<fabric::ToolDefinition>| {
            definitions
                .into_iter()
                .map(|definition| definition.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(reg.definitions()), ["alpha", "middle", "zeta"]);
        assert_eq!(
            names(reg.profile_definitions()),
            ["alpha", "middle", "zeta"]
        );
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
