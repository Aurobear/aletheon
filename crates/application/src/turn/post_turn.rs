//! Post-turn projection contracts for an already-settled turn.

use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PostTurnOutcome {
    pub session_id: String,
    pub principal_id: contracts::PrincipalId,
    pub input: String,
    pub output: String,
    pub turn: usize,
    pub succeeded: bool,
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
    pub agora_start_version: u64,
}

#[async_trait]
pub trait PostTurnProjection: Send + Sync {
    async fn project(&self, outcome: PostTurnOutcome) -> anyhow::Result<()>;
}

pub struct PostTurnDispatch {
    pub projector: Arc<dyn PostTurnProjection>,
    pub outcome: PostTurnOutcome,
}

#[async_trait]
pub trait PostTurnRuntimePort: Send + Sync {
    async fn post_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()>;
}

#[async_trait]
pub trait TurnPostEffectsPort: Send + Sync {
    async fn complete_policy(
        &self,
        turn_count: usize,
        input: &str,
        result: &contracts::TurnResult,
        succeeded: bool,
    );
    async fn observe_assistant(&self, output: &str);
    async fn after_terminal(&self, succeeded: bool) -> anyhow::Result<()>;
}

pub fn bounded_summary(input: &str, max_chars: usize) -> String {
    let end = input
        .char_indices()
        .nth(max_chars)
        .map_or(input.len(), |(index, _)| index);
    if end < input.len() {
        format!("{}...", &input[..end])
    } else {
        input.to_owned()
    }
}
