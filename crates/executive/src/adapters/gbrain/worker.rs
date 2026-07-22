//! Executive supervisor for Mnemosyne-owned GBrain reconciliation.

use std::sync::Arc;
use std::time::Duration;

use mnemosyne::supplemental::{
    RetryPolicy, SpoolError, SupplementalMemoryTransport, SupplementalReconciliationService,
    SupplementalSpool,
};
use mnemosyne::RetentionRepository;
use tokio_util::sync::CancellationToken;

pub use mnemosyne::supplemental::ReconciliationDrainReport as DrainReport;

/// Scheduling-only adapter. All claim, receipt, retry, dead-letter, and
/// tombstone-settlement decisions live in Mnemosyne.
pub struct GbrainWorker<T: SupplementalMemoryTransport> {
    service: SupplementalReconciliationService<T>,
}

impl<T: SupplementalMemoryTransport> GbrainWorker<T> {
    pub fn new(
        spool: Arc<SupplementalSpool>,
        transport: Arc<T>,
        retry: RetryPolicy,
        worker_id: impl Into<String>,
        batch_size: usize,
        lease_ms: i64,
    ) -> Result<Self, SpoolError> {
        Ok(Self {
            service: SupplementalReconciliationService::new(
                spool, transport, retry, worker_id, batch_size, lease_ms,
            )?,
        })
    }

    pub fn with_retention_repository(mut self, retention: Arc<RetentionRepository>) -> Self {
        self.service = self.service.with_retention_repository(retention);
        self
    }

    pub async fn drain_once(
        &self,
        now_ms: i64,
        cancel: &CancellationToken,
    ) -> Result<DrainReport, SpoolError> {
        self.service.drain_once(now_ms, cancel).await
    }

    pub async fn run(
        &self,
        clock: Arc<dyn fabric::Clock>,
        interval: Duration,
        cancel: CancellationToken,
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let now_ms = clock.wall_now().0.max(0);
            if let Err(error) = self.service.drain_once(now_ms, &cancel).await {
                tracing::warn!(error = %error, "GBrain reconciliation drain degraded");
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
            }
        }
    }
}
