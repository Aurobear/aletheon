//! Aletheon compatibility adapter for Runtime Agent admission.

use kernel::BudgetController;

use std::sync::Arc;

use ::contracts::{AgentControlError, AgentSpawnRequest};
use async_trait::async_trait;
use cognit::config::AgentAdmissionConfig;
use kernel::admission::InMemoryBudgetController;

pub use runtime::{
    AgentAdmissionLease, AgentAdmissionMetrics, AgentAdmissionPort, AgentAdmissionRequest,
    AgentStorageRequest,
};

pub(super) fn constrain_cognitive_workspace(
    request: &mut AgentSpawnRequest,
) -> Result<(), AgentControlError> {
    runtime::constrain_cognitive_workspace(request)
}

/// Compatibility shell preserving the Aletheon constructor/configuration
/// surface while Runtime owns all admission state, fairness, and settlement.
pub struct BoundedAgentAdmission {
    inner: runtime::BoundedAgentAdmission,
}

impl std::fmt::Debug for BoundedAgentAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(formatter)
    }
}

impl BoundedAgentAdmission {
    /// Compatibility constructor used by existing Aletheon fixtures.
    pub fn new(max_concurrent: usize) -> Result<Self, AgentControlError> {
        let config = AgentAdmissionConfig {
            max_agents_per_root: max_concurrent,
            max_running_agents: max_concurrent,
            max_queued_per_root: max_concurrent,
            ..AgentAdmissionConfig::default()
        };
        Self::with_budget(config, Arc::new(InMemoryBudgetController::new()))
    }

    pub fn with_budget(
        config: AgentAdmissionConfig,
        budget: Arc<dyn BudgetController>,
    ) -> Result<Self, AgentControlError> {
        config
            .validate()
            .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        let policy = runtime::AgentAdmissionPolicy {
            max_agents_per_root: config.max_agents_per_root,
            max_running_agents: config.max_running_agents,
            max_depth: config.max_depth,
            max_queued_per_root: config.max_queued_per_root,
            sibling_fairness_quantum: config.sibling_fairness_quantum,
            root_max_tokens: config.root_max_tokens,
            root_max_cost_micro: config.root_max_cost_micro,
            max_child_tokens: config.max_child_tokens,
            max_child_cost_micro: config.max_child_cost_micro,
            max_storage_bytes: config.max_storage_bytes,
            max_storage_items: config.max_storage_items,
        };
        Ok(Self {
            inner: runtime::BoundedAgentAdmission::with_budget(policy, budget)?,
        })
    }

    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }
}

#[async_trait]
impl AgentAdmissionPort for BoundedAgentAdmission {
    async fn reserve(
        &self,
        request: AgentAdmissionRequest<'_>,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError> {
        self.inner.reserve(request).await
    }

    fn metrics(&self) -> AgentAdmissionMetrics {
        self.inner.metrics()
    }
}
