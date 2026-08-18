use std::sync::Arc;

use async_trait::async_trait;
use runtime::EventSpine;

pub struct AgentEvaluationProjectionSink {
    inner: crate::adapters::post_turn::DurableDomainEvaluationSink,
}

impl AgentEvaluationProjectionSink {
    pub fn new(spine: Arc<dyn EventSpine>) -> Self {
        Self {
            inner: crate::adapters::post_turn::DurableDomainEvaluationSink::new(
                "agent_control",
                spine,
            ),
        }
    }
}

#[async_trait]
impl application::evaluation_projection::EvaluationProjectionSink
    for AgentEvaluationProjectionSink
{
    fn name(&self) -> &'static str {
        "agent_control"
    }

    async fn project(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> anyhow::Result<()> {
        application::evaluation_projection::EvaluationProjectionSink::project(&self.inner, record)
            .await
    }
}
