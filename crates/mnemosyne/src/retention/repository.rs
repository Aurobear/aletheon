use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::backends::supplemental::SupplementalDocument;
use crate::model::{MemoryKind, MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus};
use crate::observability::{MemoryMetrics, TombstoneDestination};
use crate::service::{ForgetAuthority, ForgetPolicy, ForgetReceipt, ForgetSelector};

pub struct RetentionRepository {
    connection: Mutex<Connection>,
    metrics: Mutex<MemoryMetrics>,
}

impl RetentionRepository {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS retention_records(
               record_id TEXT PRIMARY KEY,
               scope_json TEXT NOT NULL,
               kind_json TEXT NOT NULL,
               record_json TEXT,
               status TEXT NOT NULL,
               recorded_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_retention_scope ON retention_records(scope_json,status);
             CREATE TABLE IF NOT EXISTS forget_previews(
               request_id TEXT PRIMARY KEY,
               policy_hash TEXT NOT NULL,
               previewed_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS forget_requests(
               request_id TEXT PRIMARY KEY,
               policy_hash TEXT NOT NULL,
               receipt_json TEXT NOT NULL,
               requested_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_tombstones(
               record_id TEXT PRIMARY KEY,
               request_id TEXT NOT NULL,
               requester TEXT NOT NULL,
               reason TEXT NOT NULL,
               authority TEXT NOT NULL,
               requested_ms INTEGER NOT NULL,
               remote_state TEXT NOT NULL CHECK(remote_state IN ('not_required','pending','settled')),
               payload_removed_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_tombstone_compaction ON memory_tombstones(payload_removed_ms,requested_ms);
             CREATE TABLE IF NOT EXISTS retention_compaction_state(
               singleton INTEGER PRIMARY KEY CHECK(singleton=1),
               lease_owner TEXT,
               lease_until_ms INTEGER,
               watermark TEXT,
               last_compacted_ms INTEGER
             );
             INSERT OR IGNORE INTO retention_compaction_state(singleton) VALUES(1);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            metrics: Mutex::new(MemoryMetrics::default()),
        })
    }

    pub fn set_metrics(&self, metrics: MemoryMetrics) {
        *self
            .metrics
            .lock()
            .expect("retention metrics mutex poisoned") = metrics;
        let _ = self.refresh_pending_metrics();
    }

    pub fn register(&self, record: &MemoryRecord, now_ms: i64) -> anyhow::Result<()> {
        record.validate()?;
        let connection = self.connection.lock().expect("retention mutex poisoned");
        connection.execute(
            "INSERT INTO retention_records(record_id,scope_json,kind_json,record_json,status,recorded_ms)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(record_id) DO UPDATE SET scope_json=excluded.scope_json,kind_json=excluded.kind_json,
               record_json=CASE WHEN retention_records.status='tombstoned' THEN retention_records.record_json ELSE excluded.record_json END,
               status=CASE WHEN retention_records.status='tombstoned' THEN retention_records.status ELSE excluded.status END",
            params![record.id.0, serde_json::to_string(&record.scope)?, serde_json::to_string(&record.kind)?, serde_json::to_string(record)?, status_name(record.status), now_ms],
        )?;
        Ok(())
    }

    pub fn preview_forget(
        &self,
        policy: &ForgetPolicy,
        now_ms: i64,
    ) -> anyhow::Result<ForgetReceipt> {
        policy.validate()?;
        let hash = policy_hash(policy)?;
        let mut connection = self.connection.lock().expect("retention mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = evaluate(&tx, policy)?;
        tx.execute(
            "INSERT OR REPLACE INTO forget_previews(request_id,policy_hash,previewed_ms) VALUES(?1,?2,?3)",
            params![policy.request_id, hash, now_ms],
        )?;
        tx.commit()?;
        drop(connection);
        self.refresh_pending_metrics()?;
        Ok(receipt)
    }

    pub fn forget(&self, policy: &ForgetPolicy, now_ms: i64) -> anyhow::Result<ForgetReceipt> {
        policy.validate()?;
        let hash = policy_hash(policy)?;
        let mut connection = self.connection.lock().expect("retention mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT policy_hash,receipt_json FROM forget_requests WHERE request_id=?1",
                [&policy.request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((stored_hash, receipt)) = prior {
            anyhow::ensure!(
                stored_hash == hash,
                "forget request ID was reused with another policy"
            );
            return Ok(serde_json::from_str(&receipt)?);
        }
        if policy.authority.is_elevated() {
            let preview_hash: Option<String> = tx
                .query_row(
                    "SELECT policy_hash FROM forget_previews WHERE request_id=?1",
                    [&policy.request_id],
                    |row| row.get(0),
                )
                .optional()?;
            anyhow::ensure!(
                preview_hash.as_deref() == Some(hash.as_str()),
                "elevated forget requires a matching dry-run preview"
            );
        }
        let mut receipt = evaluate(&tx, policy)?;
        let candidates = selected_ids(&tx, &policy.selector)?;
        for id in candidates {
            if receipt.denied.iter().any(|denied| denied.0 == id) {
                continue;
            }
            if receipt
                .already_tombstoned
                .iter()
                .any(|existing| existing.0 == id)
            {
                continue;
            }
            let record_json: Option<String> = tx
                .query_row(
                    "SELECT record_json FROM retention_records WHERE record_id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let Some(record_json) = record_json else {
                continue;
            };
            let mut record: MemoryRecord = serde_json::from_str(&record_json)?;
            record.status = MemoryStatus::Tombstoned;
            let remote_state = if SupplementalDocument::from_record(&record)?.is_some() {
                "pending"
            } else {
                "not_required"
            };
            tx.execute(
                "UPDATE retention_records SET status='tombstoned',record_json=?2 WHERE record_id=?1",
                params![id, serde_json::to_string(&record)?],
            )?;
            tx.execute(
                "INSERT INTO memory_tombstones(record_id,request_id,requester,reason,authority,requested_ms,remote_state)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![id, policy.request_id, policy.requester, policy.reason, policy.authority.name(), now_ms, remote_state],
            )?;
            receipt.tombstoned.push(MemoryRecordId(id.clone()));
            if remote_state == "pending" {
                receipt.remote_pending.push(MemoryRecordId(id));
            }
        }
        receipt.sort();
        tx.execute(
            "INSERT INTO forget_requests(request_id,policy_hash,receipt_json,requested_ms) VALUES(?1,?2,?3,?4)",
            params![policy.request_id, hash, serde_json::to_string(&receipt)?, now_ms],
        )?;
        tx.commit()?;
        drop(connection);
        self.refresh_pending_metrics()?;
        Ok(receipt)
    }

    pub fn is_tombstoned(&self, record_id: &str) -> anyhow::Result<bool> {
        Ok(self
            .connection
            .lock()
            .expect("retention mutex poisoned")
            .query_row(
                "SELECT 1 FROM retention_records WHERE record_id=?1 AND status='tombstoned'",
                [record_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn record(
        &self,
        record_id: &str,
        include_payload: bool,
    ) -> anyhow::Result<Option<MemoryRecord>> {
        if !include_payload {
            return Ok(None);
        }
        let value: Option<String> = self
            .connection
            .lock()
            .expect("retention mutex poisoned")
            .query_row(
                "SELECT record_json FROM retention_records WHERE record_id=?1",
                [record_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        value
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub fn mark_remote_settled(&self, record_id: &str) -> anyhow::Result<()> {
        self.connection
            .lock()
            .expect("retention mutex poisoned")
            .execute(
                "UPDATE memory_tombstones SET remote_state='settled' WHERE record_id=?1",
                [record_id],
            )?;
        self.refresh_pending_metrics()?;
        Ok(())
    }

    pub fn pending_remote_count(&self) -> anyhow::Result<usize> {
        Ok(self
            .connection
            .lock()
            .expect("retention mutex poisoned")
            .query_row(
                "SELECT COUNT(*) FROM memory_tombstones WHERE remote_state='pending'",
                [],
                |row| row.get(0),
            )?)
    }

    pub(crate) fn refresh_pending_metrics(&self) -> anyhow::Result<()> {
        let (local, supplemental): (usize, usize) = self
            .connection
            .lock()
            .expect("retention mutex poisoned")
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN payload_removed_ms IS NULL THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN remote_state='pending' THEN 1 ELSE 0 END),0)
                 FROM memory_tombstones",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        let metrics = self
            .metrics
            .lock()
            .expect("retention metrics mutex poisoned")
            .clone();
        metrics.set_tombstone_pending(TombstoneDestination::Local, local);
        metrics.set_tombstone_pending(TombstoneDestination::Supplemental, supplemental);
        Ok(())
    }

    pub fn pending_remote_records(&self, limit: usize) -> anyhow::Result<Vec<MemoryRecord>> {
        anyhow::ensure!(
            limit > 0 && limit <= ForgetPolicy::MAX_RECORDS,
            "remote tombstone batch is invalid"
        );
        let connection = self.connection.lock().expect("retention mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT r.record_json FROM memory_tombstones t JOIN retention_records r USING(record_id)
             WHERE t.remote_state='pending' AND r.record_json IS NOT NULL ORDER BY t.requested_ms,t.record_id LIMIT ?1",
        )?;
        let values = statement
            .query_map([limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .collect()
    }

    pub(crate) fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().expect("retention mutex poisoned")
    }
}

fn evaluate(
    tx: &rusqlite::Transaction<'_>,
    policy: &ForgetPolicy,
) -> anyhow::Result<ForgetReceipt> {
    let mut receipt = ForgetReceipt::default();
    for id in selected_ids(tx, &policy.selector)? {
        let row: Option<(String, String, String)> = tx
            .query_row(
                "SELECT scope_json,kind_json,status FROM retention_records WHERE record_id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((scope_json, kind_json, status)) = row else {
            receipt.denied.push(MemoryRecordId(id));
            continue;
        };
        let scope: MemoryScope = serde_json::from_str(&scope_json)?;
        let kind: MemoryKind = serde_json::from_str(&kind_json)?;
        let outside_boundary =
            matches!(&policy.selector, ForgetSelector::Exact { within, .. } if within != &scope);
        if outside_boundary || (elevated_record(&scope, kind) && !policy.authority.is_elevated()) {
            receipt.denied.push(MemoryRecordId(id));
        } else if status == "tombstoned" {
            receipt.already_tombstoned.push(MemoryRecordId(id));
        }
    }
    receipt.sort();
    Ok(receipt)
}

fn selected_ids(
    tx: &rusqlite::Transaction<'_>,
    selector: &ForgetSelector,
) -> anyhow::Result<Vec<String>> {
    match selector {
        ForgetSelector::Exact { record_ids, within } => {
            let _ = within;
            Ok(record_ids.iter().map(|id| id.0.clone()).collect())
        }
        ForgetSelector::Scope { scope, limit } => {
            let mut statement = tx.prepare(
                "SELECT record_id FROM retention_records WHERE scope_json=?1 ORDER BY record_id LIMIT ?2"
            )?;
            let rows = statement
                .query_map(
                    params![serde_json::to_string(scope)?, *limit as i64],
                    |row| row.get(0),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }
}

fn elevated_record(scope: &MemoryScope, kind: MemoryKind) -> bool {
    matches!(scope, MemoryScope::Principal(_) | MemoryScope::Global)
        || kind == MemoryKind::CoreState
}

fn policy_hash(policy: &ForgetPolicy) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(policy)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn status_name(status: MemoryStatus) -> &'static str {
    match status {
        MemoryStatus::Candidate => "candidate",
        MemoryStatus::Current => "current",
        MemoryStatus::Superseded => "superseded",
        MemoryStatus::Expired => "expired",
        MemoryStatus::Rejected => "rejected",
        MemoryStatus::Tombstoned => "tombstoned",
    }
}

impl ForgetAuthority {
    pub(crate) fn is_elevated(&self) -> bool {
        matches!(self, Self::Elevated { .. })
    }
    pub(crate) fn name(&self) -> &'static str {
        if self.is_elevated() {
            "elevated"
        } else {
            "ordinary"
        }
    }
}
