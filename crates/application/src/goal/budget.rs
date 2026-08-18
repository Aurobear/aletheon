//! Application-owned durable Goal budget request and reservation data.

use contracts::GoalId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBudgetRequest {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBudgetReservation {
    pub reservation_id: String,
    pub goal_id: GoalId,
    pub request: GoalBudgetRequest,
    pub status: String,
    pub created_at: String,
}
