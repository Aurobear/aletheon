//! Non-authoritative fan-out of a durable evaluation receipt reference.
//!
//! The full receipt and evidence remain in the evaluation store. Consumers see
//! only the immutable reference plus host-observed correlation fields, so no
//! projection can recompute or replace the settled decision.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const MAX_PROJECTION_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationProjectionMetrics {
    pub elapsed_ms: Option<u64>,
    pub inference_rounds: Option<u64>,
    pub provider_retries: Option<u64>,
    pub tool_calls: Option<u64>,
    pub tool_errors: Option<u64>,
    pub cumulative_input_tokens: Option<u64>,
    pub cumulative_output_tokens: Option<u64>,
    pub active_context_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationProjectionContext {
    pub session_id: String,
    pub runtime_id: String,
    pub profile_id: String,
    #[serde(default)]
    pub effective_model_id: String,
    #[serde(default)]
    pub model_display_name: String,
    #[serde(default)]
    pub workspace_boundary_sha256: String,
    #[serde(default)]
    pub verification_selection_sha256: String,
    pub rubric_id: String,
    pub rubric_version: u32,
    pub process_id: ::contracts::ProcessId,
    pub metrics: EvaluationProjectionMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationProjectionRecord {
    pub receipt: ::contracts::EvaluationReceiptRef,
    pub context: EvaluationProjectionContext,
}

#[async_trait]
pub trait EvaluationProjectionSink: Send + Sync {
    fn name(&self) -> &'static str;
    async fn project(&self, record: &EvaluationProjectionRecord) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvaluationProjectionReport {
    pub observed_by: Vec<&'static str>,
    pub failures: Vec<(&'static str, String)>,
}

#[derive(Default)]
pub struct EvaluationProjection {
    sinks: Vec<Arc<dyn EvaluationProjectionSink>>,
}

impl EvaluationProjection {
    pub fn new(sinks: Vec<Arc<dyn EvaluationProjectionSink>>) -> Self {
        Self { sinks }
    }

    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Fan out independently. A failed sink is retried with a fixed bound and
    /// never prevents the remaining sinks from observing the receipt.
    pub async fn project(&self, record: EvaluationProjectionRecord) -> EvaluationProjectionReport {
        let mut report = EvaluationProjectionReport::default();
        for sink in &self.sinks {
            let mut last_error = None;
            for attempt in 1..=MAX_PROJECTION_ATTEMPTS {
                match sink.project(&record).await {
                    Ok(()) => {
                        report.observed_by.push(sink.name());
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error.to_string());
                        if attempt < MAX_PROJECTION_ATTEMPTS {
                            tokio::task::yield_now().await;
                        }
                    }
                }
            }
            if let Some(error) = last_error {
                tracing::warn!(
                    sink = sink.name(),
                    receipt_id = %record.receipt.receipt_id.0,
                    %error,
                    "evaluation receipt projection exhausted bounded retries"
                );
                report.failures.push((sink.name(), error));
            }
        }
        report
    }
}
