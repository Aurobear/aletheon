//! Gateway projection adapter for typed Goal attempt outcomes.

use application::goal::{AttemptCoordinationOutcome, GoalProgressPort};
use application::goal_retry::RetryDecision;
use async_trait::async_trait;
use gateway::ports::{GoalProgress, GoalProgressKind};
use tokio::sync::mpsc;

pub struct GatewayGoalProgressAdapter {
    sender: mpsc::Sender<GoalProgress>,
}

impl GatewayGoalProgressAdapter {
    pub fn new(sender: mpsc::Sender<GoalProgress>) -> Self {
        Self { sender }
    }
}

#[async_trait]
impl GoalProgressPort for GatewayGoalProgressAdapter {
    async fn publish(&self, outcome: &AttemptCoordinationOutcome) {
        let _ = self.sender.send(from_outcome(outcome)).await;
    }
}

pub fn from_outcome(outcome: &AttemptCoordinationOutcome) -> GoalProgress {
    match outcome {
        AttemptCoordinationOutcome::Succeeded { attempt, .. } => GoalProgress {
            goal_id: attempt.goal_id,
            attempt_id: attempt.id,
            kind: GoalProgressKind::Succeeded,
        },
        AttemptCoordinationOutcome::Failed {
            attempt, decision, ..
        } => GoalProgress {
            goal_id: attempt.goal_id,
            attempt_id: attempt.id,
            kind: match decision {
                RetryDecision::RetrySame { .. } => GoalProgressKind::RetryBackoff,
                RetryDecision::Escalate { .. } => GoalProgressKind::Escalated,
                RetryDecision::AwaitHuman { .. } => GoalProgressKind::AwaitingHuman,
                RetryDecision::Fail { .. } => GoalProgressKind::Failed,
                RetryDecision::Cancel => GoalProgressKind::Cancelled,
            },
        },
    }
}
