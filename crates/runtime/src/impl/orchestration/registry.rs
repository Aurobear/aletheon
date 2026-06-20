use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::info;

use super::agent::Agent;
use super::config_agent::ConfigAgent;
use crate::r#impl::agent::{AgentProcess, AgentProcessConfig};
use base::agent::Pid;
use base::evolution::CognitivePulseEvent;
use base::EventBus;
use corpus::r#impl::tools::Tool;
use cognit::r#impl::llm::LlmProvider;

/// Registry of available agents and running processes.
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Arc<RwLock<dyn Agent>>>>,
    processes: RwLock<HashMap<Pid, Arc<Mutex<AgentProcess>>>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            processes: RwLock::new(HashMap::new()),
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

    // --- Process table methods ---

    /// Spawn a new AgentProcess, start it, and register it in the process table.
    pub async fn spawn_process(
        &self,
        task: String,
        config: AgentProcessConfig,
        bus: Arc<dyn EventBus>,
    ) -> anyhow::Result<Pid> {
        let mut process = AgentProcess::new(None, task, bus, config);
        process.start().await?;
        let pid = process.pid;
        info!(%pid, "Spawning agent process");
        self.processes
            .write()
            .await
            .insert(pid, Arc::new(Mutex::new(process)));
        Ok(pid)
    }

    /// Dispatch a cognitive pulse to all running processes.
    pub async fn dispatch_pulse(&self, pulse: &CognitivePulseEvent) {
        let processes = self.processes.read().await;
        for (pid, proc_arc) in processes.iter() {
            let mut proc = proc_arc.lock().await;
            if let Err(e) = proc.on_pulse(pulse).await {
                tracing::warn!(%pid, error = %e, "Process failed to handle pulse");
            }
        }
    }

    /// Get a handle to a running process by PID.
    pub async fn get_process(&self, pid: &Pid) -> Option<Arc<Mutex<AgentProcess>>> {
        self.processes.read().await.get(pid).cloned()
    }

    /// Count running processes.
    pub async fn process_count(&self) -> usize {
        self.processes.read().await.len()
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
                        registry.register(Arc::new(RwLock::new(agent))).await;
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
