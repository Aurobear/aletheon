//! Mnemosyne-owned GBrain reconciliation policy and durable receipt contracts.

use serde::{Deserialize, Serialize};

use super::page::GbrainPage;
use super::spool::{EnqueueOutcome, GbrainSpool, SpoolError};
use crate::model::{MemoryRecord, MemoryRecordId, MemoryStatus};

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

pub struct GbrainReconciliation<'a> {
    spool: &'a GbrainSpool,
}

impl<'a> GbrainReconciliation<'a> {
    pub fn new(spool: &'a GbrainSpool) -> Self {
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
        let Some(page) = GbrainPage::from_record(record)
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
            record.metadata.sensitivity.clone(),
            now_ms,
        )
    }
}
