use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use aletheon_brain::r#impl::llm::LlmProvider;
use aletheon_body::r#impl::tools::Tool;
use super::agent::Agent;
use super::config_agent::ConfigAgent;

/// Registry of available agents.
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Arc<RwLock<dyn Agent>>>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Register an agent.
    pub async fn register(&self, agent: Arc<RwLock<dyn Agent>>) {
        let id = agent.read().await.id().to_string();
        info!(agent_id = %id, "Registering agent");
        self.agents.write().await.insert(id, agent);
    }

    /// Get an agent by ID.
    pub async fn get(&self, id: &str) -> Option<Arc<RwLock<dyn Agent>>> {
        self.agents.read().await.get(id).cloned()
    }

    /// List all registered agent IDs.
    pub async fn list_ids(&self) -> Vec<String> {
        self.agents.read().await.keys().cloned().collect()
    }

    /// Find agents matching a capability.
    pub async fn find_by_capability(&self, capability_name: &str) -> Vec<String> {
        let agents = self.agents.read().await;
        let mut result = Vec::new();
        for (id, agent) in agents.iter() {
            let agent_read = agent.read().await;
            if agent_read
                .capabilities()
                .iter()
                .any(|c| c.name == capability_name)
            {
                result.push(id.clone());
            }
        }
        result
    }

    /// Find agents with tools matching a pattern.
    ///
    /// Supports trailing wildcard: `"bash*"` matches `"bash_exec"`.
    /// An exact match is used when no wildcard is present.
    pub async fn find_by_tool_pattern(&self, pattern: &str) -> Vec<String> {
        let agents = self.agents.read().await;
        let mut result = Vec::new();
        for (id, agent) in agents.iter() {
            let agent_read = agent.read().await;
            let has_tool = agent_read.tools().iter().any(|t| {
                let name = t.name();
                if pattern.ends_with('*') {
                    name.starts_with(&pattern[..pattern.len() - 1])
                } else {
                    name == pattern
                }
            });
            if has_tool {
                result.push(id.clone());
            }
        }
        result
    }

    /// Count registered agents.
    pub async fn count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Load agents from TOML config files in a directory.
    ///
    /// Scans `agents_dir` for `*.toml` files. Each file defines one agent
    /// via a `[agent]` table. Tools are filtered from `all_tools` based on
    /// the config. Each agent gets its own LLM provider via `llm_factory`.
    pub async fn load_from_config(
        agents_dir: &Path,
        all_tools: &[Box<dyn Tool>],
        llm_factory: &dyn Fn() -> anyhow::Result<Box<dyn LlmProvider>>,
    ) -> Self {
        let registry = Self::new();

        if !agents_dir.exists() {
            info!(path = %agents_dir.display(), "Agents directory not found, skipping config loading");
            return registry;
        }

        let entries = match std::fs::read_dir(agents_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(path = %agents_dir.display(), error = %e, "Failed to read agents directory");
                return registry;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "toml") {
                let llm = match llm_factory() {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to create LLM for agent");
                        continue;
                    }
                };
                match ConfigAgent::load(&path, all_tools, llm) {
                    Ok(agent) => {
                        let id = agent.id().to_string();
                        info!(agent_id = %id, path = %path.display(), "Loaded agent from config");
                        registry
                            .register(Arc::new(RwLock::new(agent)))
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "Failed to load agent from config"
                        );
                    }
                }
            }
        }

        registry
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
