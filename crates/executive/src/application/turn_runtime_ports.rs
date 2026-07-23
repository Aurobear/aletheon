//! Synchronous, turn-blocking runtime ports.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{Context, LlmProvider, ToolDefinition, Verdict};

use crate::application::governed_capability::{CapabilityExecutionContext, TurnCapabilityInvoker};

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
    async fn current(&self, session_id: &str) -> anyhow::Result<(String, usize)>;
    async fn begin_user(&self, session_id: &str, message: &str) -> anyhow::Result<(String, usize)>;
    async fn finish(
        &self,
        session_id: &str,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize>;
}

#[async_trait]
pub trait TurnConfigPort: Send + Sync {
    async fn config(&self) -> crate::composition::config::ExecutiveConfig;
}

pub trait TurnObservabilityPort: Send + Sync {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64);
}

/// Immutable authorization + behavior snapshot resolved once per turn.
/// Carries the full agent profile (prompt, model, budget, approval, tools)
/// so the main turn does not silently fall back to hardcoded defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTurnProfile {
    pub profile_name: String,
    pub allowed_tools: HashSet<String>,
    pub system_prompt: String,
    pub model_policy: Option<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub approval_policy: fabric::AgentApprovalPolicy,
    pub tool_timeout_ms: u64,
}

#[async_trait]
pub trait ActiveAgentProfilePort: Send + Sync {
    async fn snapshot(&self) -> anyhow::Result<ResolvedTurnProfile>;
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
