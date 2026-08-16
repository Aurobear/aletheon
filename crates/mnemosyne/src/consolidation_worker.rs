//! Cancellation-aware supervisor for durable memory consolidation and fact promotion.

use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct MemoryConsolidationWorker {
    service: Arc<dyn crate::MemoryService>,
    interval: Duration,
    max_backoff: Duration,
    promote_confidence: Option<f64>,
    promote_max: usize,
}

impl MemoryConsolidationWorker {
    pub fn new(service: Arc<dyn crate::MemoryService>) -> Self {
        Self {
            service,
            interval: Duration::from_secs(60),
            max_backoff: Duration::from_secs(15 * 60),
            promote_confidence: None,
            promote_max: 20,
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    pub fn with_promotion(mut self, min_confidence: f64, max_count: usize) -> Self {
        self.promote_confidence = Some(min_confidence);
        self.promote_max = max_count;
        self
    }

    pub async fn run(self, cancel: CancellationToken) {
        let mut backoff = self.interval;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {
                    match self.service.consolidate(crate::MemoryScope::Global).await {
                        Ok(()) => {
                            backoff = self.interval;
                            if let Some(min_confidence) = self.promote_confidence {
                                if let Err(error) = self.service.promote_facts(min_confidence, self.promote_max).await {
                                    tracing::warn!(%error, "fact promotion degraded");
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "memory consolidation worker degraded");
                            backoff = (backoff * 2).min(self.max_backoff);
                        }
                    }
                }
            }
        }
    }
}
