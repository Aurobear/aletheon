//! Synchronous, turn-blocking runtime ports.

use std::collections::HashSet;
use std::sync::Arc;

use ::contracts::Context;
use ::contracts::{LlmProvider, ToolDefinition};
use async_trait::async_trait;
use corpus::hook::{HookContext, HookResult};
use dasein::Verdict;

use crate::wiring::application::governed_capability::{
    CapabilityExecutionContext, TurnCapabilityInvoker,
};

#[async_trait]
pub trait TurnHookPort: Send + Sync {
    async fn execute(&self, context: HookContext) -> HookResult;
}

#[async_trait]
pub trait StormStatePort: Send + Sync {
    async fn reset(&self);
    async fn failure_count(&self) -> usize;
}

#[async_trait]
pub trait ModelSelectionPort: Send + Sync {
    async fn select(&self, message: &str) -> Arc<dyn LlmProvider>;
}

#[async_trait]
pub trait SelfPolicyPort: Send + Sync {
    async fn review(&self, intent: &dasein::Intent, context: &Context) -> anyhow::Result<Verdict>;
    async fn narrate(&self, event: &str, reason: &str);
    async fn coordinate(
        &self,
        turn: usize,
        output: &str,
        status: ::contracts::dasein::OutcomeStatus,
    );
    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync>;
}

#[async_trait]
pub trait TurnSessionStatePort: Send + Sync {
    async fn current(&self, session_id: &str) -> anyhow::Result<(String, usize)>;
    async fn begin_user(
        &self,
        session_id: &str,
        message: &str,
        model: Arc<dyn LlmProvider>,
        profile: ResolvedTurnProfile,
        context_costs: TurnContextBudgetCosts,
    ) -> anyhow::Result<BeginUserResult>;
    async fn finish(
        &self,
        session_id: &str,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
        model: Arc<dyn LlmProvider>,
        profile: ResolvedTurnProfile,
        context_costs: TurnContextBudgetCosts,
    ) -> anyhow::Result<FinishUserResult>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnContextBudgetCosts {
    pub system_and_skill_tokens: ::contracts::ContextCostTokens,
    pub pending_input_tokens: ::contracts::HistoryTokens,
}

impl TurnContextBudgetCosts {
    pub const fn without_pending_input(self) -> Self {
        Self {
            system_and_skill_tokens: self.system_and_skill_tokens,
            pending_input_tokens: ::contracts::HistoryTokens::new(0),
        }
    }
}

pub struct BeginUserResult {
    pub session_id: String,
    pub turn_count: usize,
    pub history_budget_tokens: ::contracts::HistoryBudgetTokens,
    pub context_budget: ::contracts::ContextBudgetProjection,
    pub compactions: Vec<::contracts::ContextCompactionProjection>,
    /// Monotonic process-local version of deliberate history compaction or
    /// rewrite for this canonical Session projection.
    pub rewrite_version: u64,
}

pub struct FinishUserResult {
    pub turn_count: usize,
    pub compactions: Vec<::contracts::ContextCompactionProjection>,
}

#[async_trait]
pub trait TurnConfigPort: Send + Sync {
    async fn config(&self) -> crate::config::CognitiveRuntimeConfig;
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
    pub delegated_tools: HashSet<String>,
    pub system_prompt: String,
    pub model_policy: Option<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    /// Exact serialized tool-schema estimate resolved with this immutable
    /// profile snapshot. It must not be read from mutable active-profile state.
    pub tool_schema_tokens: ::contracts::ContextCostTokens,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub approval_policy: ::contracts::AgentApprovalPolicy,
    pub tool_timeout_ms: u64,
}

#[async_trait]
pub trait ActiveAgentProfilePort: Send + Sync {
    async fn snapshot(&self) -> anyhow::Result<ResolvedTurnProfile>;
}

#[derive(Clone, Debug)]
pub struct ApprovalNotice {
    pub approval_id: String,
    pub session_id: ::contracts::SessionId,
    pub turn_id: ::contracts::TurnId,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    pub detail: Option<String>,
    pub scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
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
        profile: ResolvedTurnProfile,
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
