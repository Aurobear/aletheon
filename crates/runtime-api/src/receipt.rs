use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionStatus {
    SucceededVerified,
    SucceededUnverified,
    FailedVerification,
    Blocked,
    BudgetExhausted,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeUsage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeReceipt {
    pub status: CompletionStatus,
    pub output: String,
    pub usage: RuntimeUsage,
    pub workspace_diff: Option<String>,
    pub errors: Vec<String>,
}
