//! AgentHarness trait -- turn-level executor abstraction.
//!
//! An AgentHarness is a provider-specific executor that can claim a
//! turn (attempt) and run it to completion. The orchestrator asks each
//! harness to bid on a context, then dispatches the winning bidder.

use async_trait::async_trait;

/// Context the orchestrator passes when asking a harness to bid.
#[derive(Debug, Clone)]
pub struct HarnessContext {
    /// Provider identifier (e.g. "anthropic", "openai", "local").
    pub provider: String,
    /// Model identifier (e.g. "claude-sonnet-4-20250514").
    pub model: String,
}

/// A harness's bid describing whether it can handle the given context.
#[derive(Debug, Clone)]
pub struct HarnessBid {
    /// Whether this harness supports the given context.
    pub supported: bool,
    /// Higher values indicate stronger preference (0 = lowest).
    pub priority: u8,
}

/// Parameters for a single attempt (turn).
pub struct AttemptParams {
    /// The user/system prompt for this attempt.
    pub prompt: String,
    /// Tool definitions available to the model.
    pub tools: Vec<base::ToolDefinition>,
    /// System prompt content.
    pub system_prompt: String,
    /// Conversation history so far.
    pub messages: Vec<base::Message>,
    /// Token budget allocated for this attempt.
    pub budget: super::budget::TokenBudget,
    /// Runtime plan governing turn limits and compaction.
    pub runtime_plan: RuntimePlan,
}

/// Runtime constraints for a sequence of turns.
#[derive(Debug, Clone)]
pub struct RuntimePlan {
    /// Maximum number of turns allowed.
    pub max_turns: u32,
    /// Wall-clock timeout in milliseconds.
    pub timeout_ms: u64,
    /// Context-window size (in tokens) at which compaction triggers.
    pub compaction_threshold: usize,
}

impl Default for RuntimePlan {
    fn default() -> Self {
        Self {
            max_turns: 10,
            timeout_ms: 30_000,
            compaction_threshold: 4096,
        }
    }
}

/// Outcome of a single attempt.
#[derive(Debug, Clone)]
pub struct AttemptResult {
    /// The model's response text.
    pub response: String,
    /// Tokens consumed during this attempt.
    pub tokens_used: u32,
    /// Number of turns executed in this attempt.
    pub turn_count: u32,
    /// Terminal status of the attempt.
    pub status: AttemptStatus,
}

/// Terminal status of an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptStatus {
    /// Attempt completed successfully.
    Complete,
    /// Attempt hit the turn limit but more work remains.
    NeedMoreTurns,
    /// Token budget was exhausted before completion.
    BudgetExhausted,
    /// An error occurred.
    Error(String),
}

/// Provider-specific turn-level executor.
///
/// Implementations wrap a concrete LLM provider (Anthropic, OpenAI, local
/// inference, etc.) and expose a uniform bid-and-run interface.
#[async_trait]
pub trait AgentHarness: Send + Sync {
    /// Evaluate whether this harness can handle the given context and at
    /// what priority.
    fn supports(&self, ctx: &HarnessContext) -> HarnessBid;

    /// Execute a single attempt with the given parameters.
    async fn run_attempt(&self, params: AttemptParams) -> AttemptResult;
}
