//! Supervised worker that drains the durable GBrain SQLite spool.

use std::sync::Arc;
use std::time::Duration;

use mnemosyne::backends::gbrain::{
    GbrainPage, GbrainSpool, RetryOutcome, RetryPolicy, SpoolError, SupplementalErrorCategory,
    SupplementalMemoryTransport,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub claimed: usize,
    pub delivered: usize,
    pub retried: usize,
    pub dead_lettered: usize,
    pub interrupted: usize,
    pub queue_depth: usize,
}

pub struct GbrainWorker<T: SupplementalMemoryTransport> {
    spool: Arc<GbrainSpool>,
    transport: Arc<T>,
    retry: RetryPolicy,
    worker_id: String,
    batch_size: usize,
    lease_ms: i64,
}

impl<T: SupplementalMemoryTransport> GbrainWorker<T> {
    pub fn new(
        spool: Arc<GbrainSpool>,
        transport: Arc<T>,
        retry: RetryPolicy,
        worker_id: impl Into<String>,
        batch_size: usize,
        lease_ms: i64,
    ) -> Result<Self, SpoolError> {
        let worker_id = worker_id.into();
        if worker_id.trim().is_empty() || batch_size == 0 || lease_ms <= 0 {
            return Err(SpoolError::Invalid("worker configuration is invalid"));
        }
        Ok(Self {
            spool,
            transport,
            retry,
            worker_id,
            batch_size,
            lease_ms,
        })
    }

    pub async fn drain_once(
        &self,
        now_ms: i64,
        cancel: &CancellationToken,
    ) -> Result<DrainReport, SpoolError> {
        let claimed = self
            .spool
            .claim(&self.worker_id, now_ms, self.lease_ms, self.batch_size)?;
        let mut report = DrainReport {
            claimed: claimed.len(),
            ..Default::default()
        };
        for item in claimed {
            if cancel.is_cancelled() {
                report.interrupted += 1;
                break;
            }
            let page = GbrainPage {
                slug: item.slug,
                content: item.content,
            };
            match self.transport.put_page(&page, cancel).await {
                Ok(receipt) => {
                    self.spool.acknowledge(
                        &item.record_id,
                        &self.worker_id,
                        now_ms,
                        receipt.as_deref(),
                    )?;
                    report.delivered += 1;
                }
                Err(error)
                    if cancel.is_cancelled()
                        || error.category == SupplementalErrorCategory::Cancelled =>
                {
                    report.interrupted += 1;
                    break;
                }
                Err(error) => {
                    let permanent = !error.category.is_transient();
                    match self.spool.retry(
                        &item.record_id,
                        &self.worker_id,
                        category_name(error.category),
                        now_ms,
                        &self.retry,
                        permanent,
                    )? {
                        RetryOutcome::Scheduled { .. } => report.retried += 1,
                        RetryOutcome::DeadLettered => report.dead_lettered += 1,
                    }
                }
            }
        }
        report.queue_depth = self.spool.queue_depth()?;
        self.transport.set_queue_depth(report.queue_depth);
        Ok(report)
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
            if let Err(error) = self.drain_once(now_ms, &cancel).await {
                tracing::warn!(error = %error, "GBrain spool drain failed");
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
            }
        }
    }
}

fn category_name(category: SupplementalErrorCategory) -> &'static str {
    match category {
        SupplementalErrorCategory::Auth => "auth",
        SupplementalErrorCategory::Schema => "schema",
        SupplementalErrorCategory::InvalidPage => "invalid_page",
        SupplementalErrorCategory::RejectedArguments => "rejected_arguments",
        SupplementalErrorCategory::Timeout => "timeout",
        SupplementalErrorCategory::Cancelled => "cancelled",
        SupplementalErrorCategory::RateLimited => "rate_limited",
        SupplementalErrorCategory::Provider => "provider",
        SupplementalErrorCategory::Transport => "transport",
        SupplementalErrorCategory::MalformedResponse => "malformed_response",
        SupplementalErrorCategory::OversizedResponse => "oversized_response",
        SupplementalErrorCategory::Unsupported => "unsupported",
    }
}
