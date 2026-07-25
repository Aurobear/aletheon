//! Crash-safe SQLite spool for asynchronous supplemental memory `put_page` delivery.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::config::RetryPolicy;
use super::migrations;
use super::page::{SupplementalDocument, MAX_PAGE_BYTES};
use super::reconcile::{ReconcileOperationKind, RemoteMemoryReceipt};
use crate::service::MemorySensitivity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolLimits {
    pub max_items: usize,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Inserted,
    AlreadyPresent,
    ExcludedSensitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedPage {
    pub record_id: String,
    pub logical_page_id: String,
    pub operation: ReconcileOperationKind,
    pub schema_version: u32,
    pub slug: String,
    pub content: String,
    pub content_hash: String,
    pub attempt: u32,
    pub lease_owner: String,
    pub lease_until_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub record_id: String,
    pub slug: String,
    pub content: String,
    pub attempts: u32,
    pub failed_ms: i64,
    pub reason_category: String,
}

struct RequeuePayload {
    slug: String,
    content: String,
    hash: String,
    bytes: u64,
    created_ms: i64,
    logical_page_id: String,
    operation: String,
    schema_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    Scheduled { next_attempt_ms: i64 },
    DeadLettered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub imported: usize,
    pub already_present: usize,
    pub rejected: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SpoolError {
    #[error("supplemental memory spool conflict for stable record identity")]
    Conflict,
    #[error("supplemental memory spool item or byte quota exceeded")]
    QuotaExceeded,
    #[error("supplemental memory spool lease is not owned by this worker")]
    LeaseMismatch,
    #[error("supplemental memory spool record was not found")]
    NotFound,
    #[error("supplemental memory spool payload is corrupt")]
    Corrupt,
    #[error("supplemental memory spool payload is invalid: {0}")]
    Invalid(&'static str),
    #[error("supplemental memory spool storage error")]
    Storage(#[from] rusqlite::Error),
    #[error("supplemental memory spool filesystem error")]
    Io(#[from] std::io::Error),
    #[error("supplemental memory legacy entry is malformed")]
    LegacyMalformed,
}

pub struct SupplementalSpool {
    path: PathBuf,
    limits: SpoolLimits,
    fault_full_once: Arc<AtomicBool>,
}

impl SupplementalSpool {
    pub fn open(path: impl AsRef<Path>, limits: SpoolLimits) -> Result<Self, SpoolError> {
        if limits.max_items == 0 || limits.max_bytes == 0 {
            return Err(SpoolError::Invalid("spool limits must be positive"));
        }
        let path = expand_home(path.as_ref());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(SpoolError::Invalid("spool path must not be a symlink"));
        }
        let spool = Self {
            path,
            limits,
            fault_full_once: Arc::new(AtomicBool::new(false)),
        };
        let connection = spool.connection()?;
        migrations::migrate(&connection)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&spool.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(spool)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Deterministic storage-failure seam used by crash-safety tests.
    #[doc(hidden)]
    pub fn inject_disk_full_once(&self) {
        self.fault_full_once.store(true, Ordering::SeqCst);
    }

    fn connection(&self) -> Result<Connection, SpoolError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    pub fn enqueue(
        &self,
        record_id: &str,
        page: &SupplementalDocument,
        sensitivity: MemorySensitivity,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, SpoolError> {
        self.enqueue_operation(
            record_id,
            &page.slug,
            ReconcileOperationKind::Upsert,
            1,
            page,
            sensitivity,
            now_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_operation(
        &self,
        record_id: &str,
        logical_page_id: &str,
        operation: ReconcileOperationKind,
        schema_version: u32,
        page: &SupplementalDocument,
        sensitivity: MemorySensitivity,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, SpoolError> {
        if matches!(
            sensitivity,
            MemorySensitivity::Confidential | MemorySensitivity::Restricted
        ) {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        }
        validate_payload(record_id, page)?;
        if logical_page_id.trim().is_empty() || logical_page_id.len() > 512 || schema_version == 0 {
            return Err(SpoolError::Invalid("reconciliation identity is invalid"));
        }
        if contains_secret(&page.content) {
            return Err(SpoolError::Invalid(
                "page content contains credential material",
            ));
        }
        let hash = payload_hash(&page.slug, &page.content);
        let payload_bytes = page.slug.len().saturating_add(page.content.len()) as u64;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((existing_id, existing_hash)) = find_existing(&tx, record_id, &page.slug)? {
            if existing_id == record_id && existing_hash == hash {
                tx.commit()?;
                return Ok(EnqueueOutcome::AlreadyPresent);
            }
            return Err(SpoolError::Conflict);
        }
        let (count, bytes): (u64, u64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes),0) FROM gbrain_pages",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if count >= self.limits.max_items as u64
            || bytes.saturating_add(payload_bytes) > self.limits.max_bytes
        {
            return Err(SpoolError::QuotaExceeded);
        }
        if self.fault_full_once.swap(false, Ordering::SeqCst) {
            return Err(SpoolError::Storage(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                Some("injected disk-full failure".into()),
            )));
        }
        tx.execute(
            "INSERT INTO gbrain_pages(record_id,slug,content,content_hash,payload_bytes,created_ms,logical_page_id,operation_kind,schema_version)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                record_id,
                page.slug,
                page.content,
                hash,
                payload_bytes,
                now_ms,
                logical_page_id,
                operation.as_str(),
                schema_version
            ],
        )?;
        tx.execute(
            "INSERT INTO gbrain_queue(record_id,state,attempts,next_attempt_ms,updated_ms)
             VALUES(?1,'pending',0,?2,?2)",
            params![record_id, now_ms],
        )?;
        tx.commit()?;
        Ok(EnqueueOutcome::Inserted)
    }

    pub fn claim(
        &self,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
        limit: usize,
    ) -> Result<Vec<ClaimedPage>, SpoolError> {
        if owner.trim().is_empty() || lease_ms <= 0 || limit == 0 {
            return Err(SpoolError::Invalid("claim arguments are invalid"));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut statement = tx.prepare(
            "SELECT p.record_id,p.logical_page_id,p.operation_kind,p.schema_version,p.slug,p.content,p.content_hash,q.attempts
             FROM gbrain_queue q JOIN gbrain_pages p ON p.record_id=q.record_id
             WHERE (q.state='pending' AND q.next_attempt_ms<=?1)
                OR (q.state='leased' AND q.lease_until_ms<=?1)
             ORDER BY q.next_attempt_ms,p.created_ms LIMIT ?2",
        )?;
        let candidates = statement
            .query_map(params![now_ms, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, u32>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        if let Some((record_id, ..)) =
            candidates
                .iter()
                .find(|(_, _, _, _, slug, content, expected_hash, _)| {
                    payload_hash(slug, content) != *expected_hash
                })
        {
            move_to_dead(&tx, record_id, now_ms, "corrupt_payload")?;
            tx.commit()?;
            return Err(SpoolError::Corrupt);
        }
        let lease_until_ms = now_ms.saturating_add(lease_ms);
        let mut claimed = Vec::with_capacity(candidates.len());
        for (
            record_id,
            logical_page_id,
            operation,
            schema_version,
            slug,
            content,
            expected_hash,
            attempts,
        ) in candidates
        {
            let attempt = attempts.saturating_add(1);
            tx.execute(
                "UPDATE gbrain_attempts SET completed_ms=?2,outcome='lease_expired'
                 WHERE attempt_id=(SELECT MAX(attempt_id) FROM gbrain_attempts WHERE record_id=?1)
                   AND completed_ms IS NULL",
                params![record_id, now_ms],
            )?;
            tx.execute(
                "UPDATE gbrain_queue SET state='leased',attempts=?2,lease_owner=?3,
                   lease_until_ms=?4,updated_ms=?5 WHERE record_id=?1",
                params![record_id, attempt, owner, lease_until_ms, now_ms],
            )?;
            tx.execute(
                "INSERT INTO gbrain_attempts(record_id,attempt_no,started_ms)
                 VALUES(?1,?2,?3)",
                params![record_id, attempt, now_ms],
            )?;
            claimed.push(ClaimedPage {
                record_id,
                logical_page_id,
                operation: ReconcileOperationKind::parse(&operation).ok_or(SpoolError::Corrupt)?,
                schema_version,
                slug,
                content,
                content_hash: expected_hash,
                attempt,
                lease_owner: owner.to_owned(),
                lease_until_ms,
            });
        }
        tx.commit()?;
        Ok(claimed)
    }

    pub fn acknowledge(
        &self,
        claim: &ClaimedPage,
        owner: &str,
        receipt: &RemoteMemoryReceipt,
    ) -> Result<(), SpoolError> {
        if !valid_receipt(&receipt.remote_id) {
            return Err(SpoolError::Invalid("delivery receipt is invalid"));
        }
        if claim.record_id != receipt.record_id
            || claim.logical_page_id != receipt.logical_page_id
            || claim.content_hash != receipt.content_hash
            || claim.schema_version != receipt.schema_version
            || claim.operation != receipt.operation
        {
            return Err(SpoolError::Conflict);
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(String, String, String, u32, String)> = tx.query_row(
            "SELECT p.slug,p.logical_page_id,p.operation_kind,p.schema_version,p.content_hash FROM gbrain_queue q JOIN gbrain_pages p USING(record_id)
             WHERE q.record_id=?1 AND q.state='leased' AND q.lease_owner=?2",
            params![claim.record_id, owner],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, u32>(3)?, row.get::<_, String>(4)?)),
        ).optional()?;
        let Some((slug, logical_page_id, operation, schema_version, hash)) = row else {
            if tx
                .query_row(
                    "SELECT 1 FROM gbrain_delivery_receipts WHERE record_id=?1",
                    [&claim.record_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                tx.commit()?;
                return Ok(());
            }
            return Err(SpoolError::LeaseMismatch);
        };
        if logical_page_id != receipt.logical_page_id
            || operation != receipt.operation.as_str()
            || schema_version != receipt.schema_version
            || hash != receipt.content_hash
        {
            return Err(SpoolError::Conflict);
        }
        tx.execute(
            "INSERT OR REPLACE INTO gbrain_delivery_receipts(record_id,slug,content_hash,delivered_ms,remote_receipt,logical_page_id,remote_id,operation_kind,schema_version,synced_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?4)",
            params![claim.record_id, slug, hash, receipt.synced_at_ms, receipt.remote_id, receipt.logical_page_id, receipt.remote_id, receipt.operation.as_str(), receipt.schema_version],
        )?;
        tx.execute(
            "UPDATE gbrain_attempts SET completed_ms=?2,outcome='delivered'
             WHERE attempt_id=(SELECT MAX(attempt_id) FROM gbrain_attempts WHERE record_id=?1)",
            params![claim.record_id, receipt.synced_at_ms],
        )?;
        tx.execute(
            "DELETE FROM gbrain_queue WHERE record_id=?1",
            [&claim.record_id],
        )?;
        tx.execute(
            "DELETE FROM gbrain_pages WHERE record_id=?1",
            [&claim.record_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn retry(
        &self,
        record_id: &str,
        owner: &str,
        category: &str,
        now_ms: i64,
        policy: &RetryPolicy,
        permanent: bool,
    ) -> Result<RetryOutcome, SpoolError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let safe_category = safe_category(category);
        let row: Option<(u32, i64)> = tx.query_row(
            "SELECT q.attempts,p.created_ms FROM gbrain_queue q JOIN gbrain_pages p USING(record_id)
             WHERE q.record_id=?1 AND q.state='leased' AND q.lease_owner=?2",
            params![record_id, owner], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((attempts, created_ms)) = row else {
            return Err(SpoolError::LeaseMismatch);
        };
        let too_old =
            now_ms.saturating_sub(created_ms) >= (policy.max_age_secs as i64).saturating_mul(1000);
        if permanent || attempts >= policy.max_attempts || too_old {
            finish_attempt(
                &tx,
                record_id,
                attempts,
                now_ms,
                "dead_letter",
                safe_category,
            )?;
            move_to_dead(&tx, record_id, now_ms, safe_category)?;
            tx.commit()?;
            return Ok(RetryOutcome::DeadLettered);
        }
        let delay = retry_delay_ms(record_id, attempts, policy);
        let next_attempt_ms = now_ms.saturating_add(delay as i64);
        finish_attempt(&tx, record_id, attempts, now_ms, "retry", safe_category)?;
        tx.execute(
            "UPDATE gbrain_queue SET state='pending',next_attempt_ms=?2,lease_owner=NULL,
               lease_until_ms=NULL,updated_ms=?3 WHERE record_id=?1",
            params![record_id, next_attempt_ms, now_ms],
        )?;
        tx.commit()?;
        Ok(RetryOutcome::Scheduled { next_attempt_ms })
    }

    pub fn dead_letters(&self, limit: usize) -> Result<Vec<DeadLetter>, SpoolError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT record_id,slug,content,attempts,failed_ms,reason_category
             FROM gbrain_dead_letters ORDER BY failed_ms LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(DeadLetter {
                    record_id: row.get(0)?,
                    slug: row.get(1)?,
                    content: row.get(2)?,
                    attempts: row.get(3)?,
                    failed_ms: row.get(4)?,
                    reason_category: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn requeue_dead_letter(&self, record_id: &str, now_ms: i64) -> Result<(), SpoolError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<RequeuePayload> = tx.query_row(
            "SELECT slug,content,content_hash,payload_bytes,created_ms,logical_page_id,operation_kind,schema_version FROM gbrain_dead_letters WHERE record_id=?1",
            [record_id], |row| Ok(RequeuePayload {
                slug: row.get(0)?, content: row.get(1)?, hash: row.get(2)?, bytes: row.get(3)?,
                created_ms: row.get(4)?, logical_page_id: row.get(5)?, operation: row.get(6)?,
                schema_version: row.get(7)?,
            }),
        ).optional()?;
        let Some(payload) = row else {
            return Err(SpoolError::NotFound);
        };
        self.ensure_quota(&tx, payload.bytes)?;
        tx.execute(
            "INSERT INTO gbrain_pages(record_id,slug,content,content_hash,payload_bytes,created_ms,logical_page_id,operation_kind,schema_version)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![record_id, payload.slug, payload.content, payload.hash, payload.bytes, payload.created_ms, payload.logical_page_id, payload.operation, payload.schema_version],
        )?;
        tx.execute("INSERT INTO gbrain_queue(record_id,state,attempts,next_attempt_ms,updated_ms) VALUES(?1,'pending',0,?2,?2)", params![record_id,now_ms])?;
        tx.execute(
            "DELETE FROM gbrain_dead_letters WHERE record_id=?1",
            [record_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn queue_depth(&self) -> Result<usize, SpoolError> {
        Ok(self
            .connection()?
            .query_row("SELECT COUNT(*) FROM gbrain_queue", [], |row| row.get(0))?)
    }

    pub fn has_receipt(&self, record_id: &str) -> Result<bool, SpoolError> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT 1 FROM gbrain_delivery_receipts WHERE record_id=?1",
                [record_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn receipt(&self, record_id: &str) -> Result<Option<RemoteMemoryReceipt>, SpoolError> {
        self.connection()?
            .query_row(
                "SELECT record_id,logical_page_id,remote_id,content_hash,operation_kind,schema_version,synced_at_ms
                 FROM gbrain_delivery_receipts WHERE record_id=?1",
                [record_id],
                |row| {
                    let operation: String = row.get(4)?;
                    Ok(RemoteMemoryReceipt {
                        record_id: row.get(0)?,
                        logical_page_id: row.get(1)?,
                        remote_id: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        content_hash: row.get(3)?,
                        operation: ReconcileOperationKind::parse(&operation)
                            .ok_or_else(|| rusqlite::Error::InvalidColumnType(4, "operation_kind".into(), rusqlite::types::Type::Text))?,
                        schema_version: row.get(5)?,
                        synced_at_ms: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(SpoolError::from)
    }

    pub fn migrate_legacy_outbox(
        &self,
        directory: impl AsRef<Path>,
        max_files: usize,
        now_ms: i64,
    ) -> Result<MigrationReport, SpoolError> {
        if !directory.as_ref().exists() {
            return Ok(MigrationReport {
                imported: 0,
                already_present: 0,
                rejected: 0,
            });
        }
        let mut paths = fs::read_dir(directory.as_ref())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut report = MigrationReport {
            imported: 0,
            already_present: 0,
            rejected: 0,
        };
        for path in paths.into_iter().take(max_files) {
            let result = (|| -> Result<EnqueueOutcome, SpoolError> {
                let bytes = fs::read(&path)?;
                if bytes.len() > MAX_PAGE_BYTES {
                    return Err(SpoolError::LegacyMalformed);
                }
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| SpoolError::LegacyMalformed)?;
                let slug = value
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(SpoolError::LegacyMalformed)?;
                let markdown = value
                    .get("markdown")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(SpoolError::LegacyMalformed)?;
                let clean = redact_legacy(markdown);
                let record_id = format!("legacy:{}", hex_digest(slug.as_bytes()));
                self.enqueue(
                    &record_id,
                    &SupplementalDocument {
                        slug: slug.to_owned(),
                        content: clean,
                    },
                    MemorySensitivity::Internal,
                    now_ms,
                )
            })();
            match result {
                Ok(EnqueueOutcome::Inserted) => report.imported += 1,
                Ok(EnqueueOutcome::AlreadyPresent) => report.already_present += 1,
                Ok(EnqueueOutcome::ExcludedSensitive)
                | Err(SpoolError::LegacyMalformed | SpoolError::Invalid(_)) => {
                    report.rejected += 1;
                    continue;
                }
                Err(error) => return Err(error),
            }
            let migrated = path.with_extension("json.migrated");
            fs::rename(&path, migrated)?;
        }
        Ok(report)
    }

    fn ensure_quota(&self, tx: &Transaction<'_>, added_bytes: u64) -> Result<(), SpoolError> {
        let (count, bytes): (u64, u64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(payload_bytes),0) FROM gbrain_pages",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if count >= self.limits.max_items as u64
            || bytes.saturating_add(added_bytes) > self.limits.max_bytes
        {
            Err(SpoolError::QuotaExceeded)
        } else {
            Ok(())
        }
    }
}

fn find_existing(
    tx: &Transaction<'_>,
    record_id: &str,
    slug: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    if let Some(row) = tx
        .query_row(
            "SELECT record_id,content_hash FROM gbrain_pages WHERE record_id=?1 OR slug=?2",
            params![record_id, slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    {
        return Ok(Some(row));
    }
    if let Some(row) = tx
        .query_row(
        "SELECT record_id,content_hash FROM gbrain_delivery_receipts WHERE record_id=?1 OR slug=?2",
        params![record_id, slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    {
        return Ok(Some(row));
    }
    tx.query_row(
        "SELECT record_id,content_hash FROM gbrain_dead_letters WHERE record_id=?1 OR slug=?2",
        params![record_id, slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

fn validate_payload(record_id: &str, page: &SupplementalDocument) -> Result<(), SpoolError> {
    if record_id.trim().is_empty() || record_id.len() > 512 {
        return Err(SpoolError::Invalid("record ID is invalid"));
    }
    if page.slug.trim().is_empty() || page.slug.len() > 512 {
        return Err(SpoolError::Invalid("slug is invalid"));
    }
    if page.content.trim().is_empty() || page.content.len() > MAX_PAGE_BYTES {
        return Err(SpoolError::Invalid("page content is invalid"));
    }
    Ok(())
}

fn payload_hash(slug: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(slug);
    hasher.update([0]);
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn retry_delay_ms(record_id: &str, attempt: u32, policy: &RetryPolicy) -> u64 {
    let shift = attempt.saturating_sub(1).min(31);
    let base = policy
        .initial_delay_ms
        .saturating_mul(1_u64 << shift)
        .min(policy.max_delay_ms);
    let digest = Sha256::digest(format!("{record_id}:{attempt}").as_bytes());
    let jitter_window = base / 4;
    let jitter = if jitter_window == 0 {
        0
    } else {
        u64::from_le_bytes(digest[..8].try_into().unwrap()) % (jitter_window + 1)
    };
    base.saturating_add(jitter).min(policy.max_delay_ms)
}

fn finish_attempt(
    tx: &Transaction<'_>,
    record_id: &str,
    attempt: u32,
    now_ms: i64,
    outcome: &str,
    category: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE gbrain_attempts SET completed_ms=?3,outcome=?4,error_category=?5
         WHERE attempt_id=(SELECT MAX(attempt_id) FROM gbrain_attempts WHERE record_id=?1 AND attempt_no=?2)",
        params![record_id, attempt, now_ms, outcome, category],
    )?;
    Ok(())
}

fn move_to_dead(
    tx: &Transaction<'_>,
    record_id: &str,
    now_ms: i64,
    category: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO gbrain_dead_letters(record_id,slug,content,content_hash,payload_bytes,attempts,created_ms,failed_ms,reason_category,logical_page_id,operation_kind,schema_version)
         SELECT p.record_id,p.slug,p.content,p.content_hash,p.payload_bytes,q.attempts,p.created_ms,?2,?3,p.logical_page_id,p.operation_kind,p.schema_version
         FROM gbrain_pages p JOIN gbrain_queue q USING(record_id) WHERE p.record_id=?1",
        params![record_id,now_ms,category],
    )?;
    tx.execute("DELETE FROM gbrain_queue WHERE record_id=?1", [record_id])?;
    tx.execute("DELETE FROM gbrain_pages WHERE record_id=?1", [record_id])?;
    Ok(())
}

fn redact_legacy(value: &str) -> String {
    let patterns = [
        r"(?i)Bearer\s+\S+",
        r"(?i)sk-\S+",
        r"(?i)(token|password|secret)\s*=\s*\S+",
    ];
    patterns.iter().fold(value.to_owned(), |current, pattern| {
        Regex::new(pattern)
            .unwrap()
            .replace_all(&current, "[REDACTED]")
            .into_owned()
    })
}

fn contains_secret(value: &str) -> bool {
    [
        r"(?i)Bearer\s+\S+",
        r"(?i)sk-[A-Za-z0-9_-]{8,}",
        r"(?i)(token|password|secret)\s*=\s*\S+",
    ]
    .iter()
    .any(|pattern| Regex::new(pattern).unwrap().is_match(value))
}

fn safe_category(value: &str) -> &str {
    if !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_:-".contains(&byte)
        })
    {
        value
    } else {
        "unspecified"
    }
}

fn valid_receipt(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.' | b'/')
        })
}

fn expand_home(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}
