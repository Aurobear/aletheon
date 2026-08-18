use std::sync::Arc;

use async_trait::async_trait;
use contracts::{ContextCompactionProjection, HistoryBudgetTokens, LlmProvider};

use crate::turn::context::TurnContextBudgetCosts;
use crate::turn::settings::{ResolvedTurnProfile, TurnRuntimeSettings};

#[async_trait]
pub trait StormStatePort: Send + Sync {
    async fn reset(&self);
    async fn failure_count(&self) -> usize;
}

#[async_trait]
pub trait ModelSelectionPort: Send + Sync {
    async fn select(&self, message: &str) -> Arc<dyn LlmProvider>;
}

pub struct BeginUserResult {
    pub session_id: String,
    pub turn_count: usize,
    pub history_budget_tokens: HistoryBudgetTokens,
    pub rewrite_version: u64,
}

pub struct FinishUserResult {
    pub turn_count: usize,
    pub compactions: Vec<ContextCompactionProjection>,
}

#[async_trait]
pub trait TurnSessionStatePort: Send + Sync {
    async fn current(&self, session_id: &str) -> anyhow::Result<(String, usize)>;
    async fn begin_user(
        &self,
        session_id: &str,
        turn_id: contracts::TurnId,
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

#[async_trait]
pub trait TurnConfigPort: Send + Sync {
    async fn config(&self) -> TurnRuntimeSettings;
}

pub trait TurnObservabilityPort: Send + Sync {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64);
}

#[async_trait]
pub trait ActiveAgentProfilePort: Send + Sync {
    async fn snapshot(&self) -> anyhow::Result<ResolvedTurnProfile>;
}

#[derive(Clone, Debug)]
pub struct ApprovalNotice {
    pub approval_id: String,
    pub session_id: contracts::SessionId,
    pub turn_id: contracts::TurnId,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    pub detail: Option<String>,
    pub scope_subject: Option<contracts::protocol::client::TransientApprovalScopeSubject>,
}

#[async_trait]
pub trait TurnApprovalPort: Send + Sync {
    async fn next(&self) -> Option<ApprovalNotice>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnOperationMetrics {
    pub active_scopes: usize,
    pub active_resources: usize,
    pub drop_fallbacks: usize,
    pub pending_reclaim: usize,
    pub reclaim_queue_exhausted: usize,
}

/// Consumer-owned boundary for the host operation lifecycle surrounding a
/// canonical Runtime Turn. Runtime remains the Turn authority; this port only
/// accounts for host execution and cancellation.
#[async_trait]
pub trait TurnOperationPort: Send + Sync {
    async fn admit(
        &self,
        owner: contracts::ProcessId,
        deadline: Option<contracts::MonoDeadline>,
    ) -> anyhow::Result<contracts::OperationId>;
    async fn succeed(&self, operation: contracts::OperationId) -> anyhow::Result<()>;
    async fn fail(&self, operation: contracts::OperationId, reason: String) -> anyhow::Result<()>;
    async fn cancel(
        &self,
        operation: contracts::OperationId,
        reason: contracts::CancelReason,
    ) -> anyhow::Result<()>;
    fn metrics(&self) -> TurnOperationMetrics;
}

#[async_trait]
pub trait TurnTimerPort: Send + Sync {
    async fn sleep(&self, duration: std::time::Duration);
}

#[async_trait]
pub trait TurnSessionPort: Send + Sync {
    async fn load_session(
        &self,
        id: &contracts::SessionId,
    ) -> anyhow::Result<Option<contracts::SessionRecord>>;
    async fn load_items(
        &self,
        session: &contracts::SessionId,
        through_sequence: Option<u64>,
    ) -> anyhow::Result<Vec<contracts::ItemRecord>>;
    async fn create(&self, session: contracts::SessionRecord) -> anyhow::Result<()>;
    async fn bind_principal(
        &self,
        session: &contracts::SessionId,
        principal: &contracts::PrincipalId,
    ) -> anyhow::Result<()>;
    async fn append(
        &self,
        session: &contracts::SessionId,
        expected_sequence: u64,
        item: contracts::ItemRecord,
    ) -> anyhow::Result<contracts::AppendOutcome>;
}

pub trait TurnIdentityPort: Send + Sync {
    fn contract_session(
        &self,
        thread: &contracts::ThreadId,
    ) -> anyhow::Result<contracts::SessionId>;
    fn runtime_session(&self, session: &contracts::SessionId)
        -> anyhow::Result<runtime::SessionId>;
    fn runtime_turn(&self, turn: contracts::TurnId) -> anyhow::Result<runtime::TurnId>;
}
