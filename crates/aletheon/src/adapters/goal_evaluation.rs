//! Durable Goal projection adapter for settled evaluation receipts.

use adapters_sqlite::goal::ObjectiveStore;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub struct GoalEvaluationProjectionSink {
    inner: crate::adapters::post_turn::DurableDomainEvaluationSink,
    store: Arc<Mutex<ObjectiveStore>>,
}

impl GoalEvaluationProjectionSink {
    pub fn new(spine: Arc<dyn runtime::EventSpine>, store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self {
            inner: crate::adapters::post_turn::DurableDomainEvaluationSink::new("goal", spine),
            store,
        }
    }
}

#[async_trait]
impl application::evaluation_projection::EvaluationProjectionSink for GoalEvaluationProjectionSink {
    fn name(&self) -> &'static str {
        "goal"
    }

    async fn project(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> anyhow::Result<()> {
        if record.receipt.subject_kind == "goal_attempt" {
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("Goal store lock poisoned"))?
                .record_evaluation_feedback(&record.receipt)?;
        }
        application::evaluation_projection::EvaluationProjectionSink::project(&self.inner, record)
            .await
    }
}
