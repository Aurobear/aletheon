//! Cancellation-aware supervisor loop for durable Mnemosyne consolidation
//! plus automatic high-confidence fact promotion into CoreMemory.
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct MemoryConsolidationWorker {
    service: Arc<dyn mnemosyne::MemoryService>,
    interval: Duration,
    max_backoff: Duration,
    /// Promotion into CoreMemory (Phase 5). None = promotion disabled.
    promote_confidence: Option<f64>,
    promote_max: usize,
}
impl MemoryConsolidationWorker {
    pub fn new(service: Arc<dyn mnemosyne::MemoryService>) -> Self {
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
    /// Enable Phase 5 fact-to-CoreMemory promotion after each consolidation.
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
                    match self.service.consolidate(mnemosyne::MemoryScope::Global).await {
                        Ok(()) => {
                            backoff = self.interval;
                            // Phase 5: promote high-confidence facts into CoreMemory.
                            if let Some(min_conf) = self.promote_confidence {
                                if let Err(error) = self.service.promote_facts(min_conf, self.promote_max).await {
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
