//! Mnemosyne-owned supplemental memory reconciliation policy and durable receipt contracts.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::page::SupplementalDocument;
use super::spool::{EnqueueOutcome, SpoolError, SupplementalSpool};
use super::{RetryOutcome, RetryPolicy, SupplementalErrorCategory, SupplementalMemoryTransport};
use crate::model::{MemoryRecord, MemoryRecordId, MemoryStatus};
use crate::{MemoryMetrics, RetentionRepository};

pub const RECONCILIATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileOperationKind {
    Upsert,
    Supersede,
    Tombstone,
}

impl ReconcileOperationKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Supersede => "supersede",
            Self::Tombstone => "tombstone",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "upsert" => Some(Self::Upsert),
            "supersede" => Some(Self::Supersede),
            "tombstone" => Some(Self::Tombstone),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconcileOperation {
    Upsert(MemoryRecordId),
    Supersede(MemoryRecordId),
    Tombstone(MemoryRecordId),
}

impl ReconcileOperation {
    pub fn kind(&self) -> ReconcileOperationKind {
        match self {
            Self::Upsert(_) => ReconcileOperationKind::Upsert,
            Self::Supersede(_) => ReconcileOperationKind::Supersede,
            Self::Tombstone(_) => ReconcileOperationKind::Tombstone,
        }
    }

    pub fn record_id(&self) -> &MemoryRecordId {
        match self {
            Self::Upsert(id) | Self::Supersede(id) | Self::Tombstone(id) => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMemoryReceipt {
    pub record_id: String,
    pub logical_page_id: String,
    pub remote_id: String,
    pub content_hash: String,
    pub operation: ReconcileOperationKind,
    pub schema_version: u32,
    pub synced_at_ms: i64,
}

pub struct SupplementalReconciliation<'a> {
    spool: &'a SupplementalSpool,
}

impl<'a> SupplementalReconciliation<'a> {
    pub fn new(spool: &'a SupplementalSpool) -> Self {
        Self { spool }
    }

    pub fn enqueue(
        &self,
        record: &MemoryRecord,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, SpoolError> {
        if matches!(
            record.metadata.sensitivity,
            crate::model::MemorySensitivity::Confidential
                | crate::model::MemorySensitivity::Restricted
        ) {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        }
        let Some(page) = SupplementalDocument::from_record(record)
            .map_err(|_| SpoolError::Invalid("record is not projectable"))?
        else {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        };
        let operation = match record.status {
            MemoryStatus::Tombstoned => ReconcileOperation::Tombstone(record.id.clone()),
            MemoryStatus::Superseded | MemoryStatus::Expired => {
                ReconcileOperation::Supersede(record.id.clone())
            }
            _ => ReconcileOperation::Upsert(record.id.clone()),
        };
        let operation_id = format!("{}:{}", record.id.0, operation.kind().as_str());
        self.spool.enqueue_operation(
            &operation_id,
            &page.slug,
            operation.kind(),
            RECONCILIATION_SCHEMA_VERSION,
            &page,
            record.metadata.sensitivity,
            now_ms,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconciliationDrainReport {
    pub claimed: usize,
    pub delivered: usize,
    pub retried: usize,
    pub dead_lettered: usize,
    pub interrupted: usize,
    pub queue_depth: usize,
}

/// Mnemosyne-owned outbound reconciliation lifecycle. Executive schedules this
/// service but never claims, acknowledges, retries, or settles memory records.
pub struct SupplementalReconciliationService<T: SupplementalMemoryTransport> {
    spool: Arc<SupplementalSpool>,
    transport: Arc<T>,
    retry: RetryPolicy,
    worker_id: String,
    batch_size: usize,
    lease_ms: i64,
    retention: Option<Arc<RetentionRepository>>,
    metrics: MemoryMetrics,
}

impl<T: SupplementalMemoryTransport> SupplementalReconciliationService<T> {
    pub fn new(
        spool: Arc<SupplementalSpool>,
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
            retention: None,
            metrics: MemoryMetrics::default(),
        })
    }

    pub fn with_metrics(mut self, metrics: MemoryMetrics) -> Self {
        metrics.set_supplemental_queue_depth(self.spool.queue_depth().unwrap_or_default());
        self.metrics = metrics;
        self
    }

    pub fn with_retention_repository(mut self, retention: Arc<RetentionRepository>) -> Self {
        self.retention = Some(retention);
        self
    }

    pub async fn drain_once(
        &self,
        now_ms: i64,
        cancel: &CancellationToken,
    ) -> Result<ReconciliationDrainReport, SpoolError> {
        if let Some(retention) = &self.retention {
            let pending = retention
                .pending_remote_records(self.batch_size)
                .map_err(|_| SpoolError::Invalid("retention tombstone outbox is unavailable"))?;
            let reconciliation = SupplementalReconciliation::new(&self.spool);
            for record in pending {
                reconciliation.enqueue(&record, now_ms)?;
            }
        }
        let claimed = self
            .spool
            .claim(&self.worker_id, now_ms, self.lease_ms, self.batch_size)?;
        let mut report = ReconciliationDrainReport {
            claimed: claimed.len(),
            ..Default::default()
        };
        for item in claimed {
            if cancel.is_cancelled() {
                self.metrics
                    .supplemental_degraded(SupplementalErrorCategory::Cancelled.into());
                report.interrupted += 1;
                break;
            }
            let page = SupplementalDocument {
                slug: item.slug.clone(),
                content: item.content.clone(),
            };
            match self.transport.put_page(&page, cancel).await {
                Ok(remote_id) => {
                    let receipt = RemoteMemoryReceipt {
                        record_id: item.record_id.clone(),
                        logical_page_id: item.logical_page_id.clone(),
                        remote_id: remote_id.unwrap_or_else(|| item.logical_page_id.clone()),
                        content_hash: item.content_hash.clone(),
                        operation: item.operation,
                        schema_version: item.schema_version,
                        synced_at_ms: now_ms,
                    };
                    // Settle the local tombstone outbox before acknowledging the
                    // spool item. If local settlement fails, the lease remains
                    // retryable and the idempotent remote write can be replayed.
                    self.settle_tombstone(&item)?;
                    self.spool.acknowledge(&item, &self.worker_id, &receipt)?;
                    report.delivered += 1;
                }
                Err(error)
                    if cancel.is_cancelled()
                        || error.category == SupplementalErrorCategory::Cancelled =>
                {
                    self.metrics
                        .supplemental_degraded(SupplementalErrorCategory::Cancelled.into());
                    report.interrupted += 1;
                    break;
                }
                Err(error) => match self.spool.retry(
                    &item.record_id,
                    &self.worker_id,
                    category_name(error.category),
                    now_ms,
                    &self.retry,
                    !error.category.is_transient(),
                )? {
                    RetryOutcome::Scheduled { .. } => {
                        self.metrics.supplemental_degraded(error.category.into());
                        report.retried += 1;
                    }
                    RetryOutcome::DeadLettered => {
                        self.metrics.supplemental_degraded(error.category.into());
                        report.dead_lettered += 1;
                    }
                },
            }
        }
        report.queue_depth = self.spool.queue_depth()?;
        self.transport.set_queue_depth(report.queue_depth);
        self.metrics
            .set_supplemental_queue_depth(report.queue_depth);
        Ok(report)
    }

    fn settle_tombstone(&self, item: &super::ClaimedPage) -> Result<(), SpoolError> {
        if item.operation == ReconcileOperationKind::Tombstone {
            if let (Some(retention), Some(record_id)) =
                (&self.retention, item.record_id.strip_suffix(":tombstone"))
            {
                retention
                    .mark_remote_settled(record_id)
                    .map_err(|_| SpoolError::Invalid("retention settlement is unavailable"))?;
            }
        }
        Ok(())
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
        SupplementalErrorCategory::Spool => "spool",
        SupplementalErrorCategory::Unsupported => "unsupported",
    }
}
