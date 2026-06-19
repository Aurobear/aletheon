pub mod budget;
pub mod fork;
pub mod harness;
pub mod process;

pub use budget::TokenBudget;
pub use fork::{AgentFork, AgentForkCompletedPayload, ForkState};
pub use harness::{
    AgentHarness, AttemptParams, AttemptResult, AttemptStatus, HarnessBid, HarnessContext,
    RuntimePlan,
};
pub use process::{AgentProcess, AgentProcessConfig, AgentState};

use aletheon_abi::runtime::{AgentInfo, AgentStatus, ScheduledTask};
use aletheon_abi::subsystem::{Subsystem, SubsystemContext, SubsystemHealth, Version};
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;

/// Manages agent registration, dispatch, and lifecycle
pub struct AgentRuntime {
    agents: RwLock<HashMap<String, AgentInfo>>,
    tasks: RwLock<HashMap<String, ScheduledTask>>,
    initialized: bool,
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
            initialized: false,
        }
    }

    /// Register an agent
    pub fn register_agent(&self, info: AgentInfo) {
        self.agents.write().insert(info.id.clone(), info);
    }

    /// Remove an agent
    pub fn unregister_agent(&self, id: &str) -> Option<AgentInfo> {
        self.agents.write().remove(id)
    }

    /// Get agent by ID
    pub fn get_agent(&self, id: &str) -> Option<AgentInfo> {
        self.agents.read().get(id).cloned()
    }

    /// Update agent status
    pub fn set_agent_status(&self, id: &str, status: AgentStatus) {
        if let Some(agent) = self.agents.write().get_mut(id) {
            agent.status = status;
        }
    }
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Subsystem for AgentRuntime {
    fn name(&self) -> &str {
        "aletheon-runtime"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        self.initialized = true;
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if self.initialized {
            SubsystemHealth::Healthy
        } else {
            SubsystemHealth::Degraded {
                reason: "Not initialized".to_string(),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.agents.write().clear();
        self.tasks.write().clear();
        self.initialized = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}
