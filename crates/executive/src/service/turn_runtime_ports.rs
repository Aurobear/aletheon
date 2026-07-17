//! Synchronous, turn-blocking runtime ports.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{Context, LlmProvider, ToolDefinition, Verdict};

use crate::service::governed_capability::{CapabilityExecutionContext, TurnCapabilityInvoker};

#[async_trait]
pub trait TurnHookPort: Send + Sync {
    async fn execute(&self, context: HookContext) -> HookResult;
}

#[async_trait]
pub trait StormStatePort: Send + Sync {
    async fn reset(&self);
    async fn failure_count(&self) -> usize;
}

pub trait ModelSelectionPort: Send + Sync {
    fn select(&self, message: &str) -> Arc<dyn LlmProvider>;
}

#[async_trait]
pub trait SelfPolicyPort: Send + Sync {
    async fn review(&self, intent: &fabric::Intent, context: &Context) -> anyhow::Result<Verdict>;
    async fn narrate(&self, event: &str, reason: &str);
    async fn coordinate(&self, turn: usize, output: &str, status: fabric::dasein::OutcomeStatus);
    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync>;
}

#[async_trait]
pub trait TurnSessionStatePort: Send + Sync {
    async fn current(&self) -> anyhow::Result<(String, usize)>;
    async fn begin_user(&self, message: &str) -> anyhow::Result<(String, usize)>;
    async fn finish(
        &self,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize>;
}

#[async_trait]
pub trait TurnConfigPort: Send + Sync {
    async fn config(&self) -> crate::core::config::ExecutiveConfig;
}

pub trait TurnObservabilityPort: Send + Sync {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64);
}

#[derive(Clone, Debug)]
pub struct ApprovalNotice {
    pub approval_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    pub detail: Option<String>,
}

#[async_trait]
pub trait TurnApprovalPort: Send + Sync {
    async fn next(&self) -> Option<ApprovalNotice>;
}

pub struct PreparedCapabilities {
    pub definitions: Vec<ToolDefinition>,
    pub invoker: Arc<dyn TurnCapabilityInvoker>,
}

#[async_trait]
pub trait GovernedTurnCapabilityPort: Send + Sync {
    async fn prepare(
        &self,
        context: CapabilityExecutionContext,
    ) -> anyhow::Result<PreparedCapabilities>;
}

pub struct TurnRuntimePorts {
    pub hooks: Arc<dyn TurnHookPort>,
    pub storm: Arc<dyn StormStatePort>,
    pub models: Arc<dyn ModelSelectionPort>,
    pub self_policy: Arc<dyn SelfPolicyPort>,
    pub approvals: Arc<dyn TurnApprovalPort>,
    pub capabilities: Arc<dyn GovernedTurnCapabilityPort>,
    pub sessions: Arc<dyn TurnSessionStatePort>,
    pub config: Arc<dyn TurnConfigPort>,
    pub observability: Arc<dyn TurnObservabilityPort>,
}
