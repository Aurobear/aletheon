use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Record of a tool call outcome for learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeRecord {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub result_summary: String,
    pub is_error: bool,
    pub user_feedback: Option<UserFeedback>,
    pub timestamp: DateTime<Utc>,
    pub context: OutcomeContext,
}

/// User feedback on the outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFeedback {
    pub rating: i8,
    pub comment: Option<String>,
}

/// Context surrounding the outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutcomeContext {
    pub preceding_errors: usize,
    pub iteration_count: usize,
    pub system_state: Option<String>,
}

/// Host-owned persistence port for learning outcomes.
pub trait OutcomeStore: Send + Sync {
    fn record(&self, outcome: &OutcomeRecord) -> anyhow::Result<()>;
    fn recent(&self, limit: usize) -> anyhow::Result<Vec<OutcomeRecord>>;
}

/// Records outcomes through an injected persistence port.
pub struct OutcomeRecorder {
    store: Arc<dyn OutcomeStore>,
}

impl OutcomeRecorder {
    pub fn new(store: Arc<dyn OutcomeStore>) -> Self {
        Self { store }
    }

    pub fn record(&self, outcome: &OutcomeRecord) -> anyhow::Result<()> {
        self.store.record(outcome)
    }

    pub fn get_recent(&self, limit: usize) -> anyhow::Result<Vec<OutcomeRecord>> {
        self.store.recent(limit)
    }
}
