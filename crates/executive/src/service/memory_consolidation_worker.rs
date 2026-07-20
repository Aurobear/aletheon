//! Cancellation-aware supervisor loop for durable Mnemosyne consolidation.
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct MemoryConsolidationWorker {
    service: Arc<dyn mnemosyne::MemoryService>,
    interval: Duration,
    max_backoff: Duration,
}
impl MemoryConsolidationWorker {
    pub fn new(service: Arc<dyn mnemosyne::MemoryService>) -> Self {
        Self {
            service,
            interval: Duration::from_secs(60),
            max_backoff: Duration::from_secs(15 * 60),
        }
    }
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
    pub async fn run(self, cancel: CancellationToken) {
        let mut backoff = self.interval;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {
                    match self.service.consolidate(mnemosyne::MemoryScope::Global).await {
                        Ok(()) => backoff = self.interval,
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
