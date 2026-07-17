use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::CanonicalMemoryEvent;

type ExtractionClaimRow = (
    i64,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    u32,
);
use crate::observability::{ConsolidationJobState, MemoryMetrics};
use crate::{MemoryKind, MemoryRecordId, MemoryScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Pending,
    Leased,
    Succeeded,
    SucceededNoOutput,
    RetryableFailure,
    PermanentFailure,
}
impl ExtractionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::SucceededNoOutput => "succeeded_no_output",
            Self::RetryableFailure => "retryable_failure",
            Self::PermanentFailure => "permanent_failure",
        }
    }
}

impl From<ExtractionStatus> for ConsolidationJobState {
    fn from(value: ExtractionStatus) -> Self {
        match value {
            ExtractionStatus::Pending => Self::Pending,
            ExtractionStatus::Leased => Self::Leased,
            ExtractionStatus::Succeeded => Self::Succeeded,
            ExtractionStatus::SucceededNoOutput => Self::SucceededNoOutput,
            ExtractionStatus::RetryableFailure => Self::RetryableFailure,
            ExtractionStatus::PermanentFailure => Self::PermanentFailure,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionJob {
    pub idempotency_key: String,
    pub session_id: String,
    pub goal_id: Option<String>,
    pub ephemeral: bool,
    pub memory_worker: bool,
    pub completed_at_ms: Option<u64>,
    pub watermark: String,
    pub created_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedExtraction {
    pub id: i64,
    pub idempotency_key: String,
    pub session_id: String,
    pub goal_id: Option<String>,
    pub watermark: String,
    pub scope: MemoryScope,
    pub attempts: u32,
    pub lease_owner: String,
    pub lease_until_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryCandidate {
    pub kind: MemoryKind,
    pub claim: String,
    pub source_event_ids: Vec<String>,
    pub confidence: f64,
    pub proposed_scope: MemoryScope,
    pub valid_from_ms: Option<i64>,
    pub valid_until_ms: Option<i64>,
    pub redaction_version: u32,
    pub content_hash: String,
}
impl MemoryCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MemoryKind,
        claim: String,
        mut source_event_ids: Vec<String>,
        confidence: f64,
        proposed_scope: MemoryScope,
        valid_from_ms: Option<i64>,
        valid_until_ms: Option<i64>,
        redaction_version: u32,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !claim.trim().is_empty() && claim.len() <= crate::MemoryRecord::MAX_CONTENT_BYTES,
            "invalid candidate claim"
        );
        anyhow::ensure!(
            !source_event_ids.is_empty()
                && source_event_ids.len() <= crate::MemoryRecord::MAX_SOURCE_EVENTS
                && source_event_ids.iter().all(|v| !v.trim().is_empty()),
            "invalid candidate source events"
        );
        source_event_ids.sort();
        source_event_ids.dedup();
        anyhow::ensure!(
            confidence.is_finite() && (0.0..=1.0).contains(&confidence),
            "invalid candidate confidence"
        );
        proposed_scope.validate()?;
        if let (Some(a), Some(b)) = (valid_from_ms, valid_until_ms) {
            anyhow::ensure!(a < b, "invalid candidate validity");
        }
        let content_hash = format!("{:x}", Sha256::digest(claim.as_bytes()));
        Ok(Self {
            kind,
            claim,
            source_event_ids,
            confidence,
            proposed_scope,
            valid_from_ms,
            valid_until_ms,
            redaction_version,
            content_hash,
        })
    }
}
#[derive(Debug, Clone)]
pub enum ExtractionCompletion {
    Succeeded { candidates: Vec<MemoryCandidate> },
    SucceededNoOutput,
    RetryableFailure { error: String, retry_at_ms: u64 },
    PermanentFailure { error: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeLease {
    pub scope: MemoryScope,
    pub owner: String,
    pub lease_until_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ConsolidatedRecord {
    pub id: MemoryRecordId,
    pub kind: MemoryKind,
    pub content: String,
    pub source_event_ids: Vec<String>,
    pub content_hash: String,
}

pub struct ConsolidationRepository {
    connection: Mutex<Connection>,
    metrics: Mutex<MemoryMetrics>,
}
impl ConsolidationRepository {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let c = Connection::open(path)?;
        c.execute_batch(super::migrations::SCHEMA)?;
        let has_scope = c
            .prepare("PRAGMA table_info(memory_extraction_jobs)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|column| column == "scope_json");
        if !has_scope {
            c.execute(
                "ALTER TABLE memory_extraction_jobs ADD COLUMN scope_json TEXT",
                [],
            )?;
        }
        Ok(Self {
            connection: Mutex::new(c),
            metrics: Mutex::new(MemoryMetrics::default()),
        })
    }

    pub fn set_metrics(&self, metrics: MemoryMetrics) {
        *self
            .metrics
            .lock()
            .expect("consolidation metrics mutex poisoned") = metrics;
        let _ = self.refresh_job_metrics();
    }

    pub(crate) fn metrics(&self) -> MemoryMetrics {
        self.metrics
            .lock()
            .expect("consolidation metrics mutex poisoned")
            .clone()
    }

    fn refresh_job_metrics(&self) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        let mut query = connection
            .prepare("SELECT status,COUNT(*) FROM memory_extraction_jobs GROUP BY status")?;
        let rows = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(query);
        drop(connection);
        let metrics = self.metrics();
        for state in [
            ConsolidationJobState::Pending,
            ConsolidationJobState::Leased,
            ConsolidationJobState::Succeeded,
            ConsolidationJobState::SucceededNoOutput,
            ConsolidationJobState::RetryableFailure,
            ConsolidationJobState::PermanentFailure,
        ] {
            metrics.set_consolidation_jobs(state, 0);
        }
        for (status, count) in rows {
            metrics.set_consolidation_jobs(parse_status(&status)?.into(), count);
        }
        Ok(())
    }
    pub fn enqueue_extraction(&self, job: &ExtractionJob) -> anyhow::Result<i64> {
        let scope = job
            .goal_id
            .as_ref()
            .map(|goal| MemoryScope::Goal(goal.clone()))
            .unwrap_or_else(|| MemoryScope::Session(job.session_id.clone()));
        self.enqueue_job(job, &scope)
    }

    fn enqueue_job(&self, job: &ExtractionJob, scope: &MemoryScope) -> anyhow::Result<i64> {
        anyhow::ensure!(
            !job.idempotency_key.trim().is_empty()
                && !job.session_id.trim().is_empty()
                && !job.watermark.trim().is_empty(),
            "invalid extraction job"
        );
        scope.validate()?;
        let c = self.connection.lock().unwrap();
        let inserted = c.execute("INSERT OR IGNORE INTO memory_extraction_jobs(idempotency_key,session_id,goal_id,ephemeral,memory_worker,completed_at_ms,status,watermark,scope_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7,?8,?9,?9)",params![job.idempotency_key,job.session_id,job.goal_id,job.ephemeral,job.memory_worker,job.completed_at_ms,job.watermark,serde_json::to_string(scope)?,job.created_at_ms])?;
        let id = c.query_row(
            "SELECT id FROM memory_extraction_jobs WHERE idempotency_key=?1",
            [&job.idempotency_key],
            |r| r.get(0),
        )?;
        drop(c);
        if inserted == 1 {
            self.refresh_job_metrics()?;
        }
        Ok(id)
    }

    /// Durably append one canonical experience and its restart-safe extraction job.
    pub fn enqueue_experience(
        &self,
        job: &ExtractionJob,
        scope: &MemoryScope,
        event: &CanonicalMemoryEvent,
    ) -> anyhow::Result<i64> {
        anyhow::ensure!(
            !event.event_id.trim().is_empty()
                && !event.kind.trim().is_empty()
                && !event.content.trim().is_empty(),
            "invalid canonical memory event"
        );
        scope.validate()?;
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO memory_extraction_jobs(idempotency_key,session_id,goal_id,ephemeral,memory_worker,completed_at_ms,status,watermark,scope_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7,?8,?9,?9)",
            params![job.idempotency_key,job.session_id,job.goal_id,job.ephemeral,job.memory_worker,job.completed_at_ms,job.watermark,serde_json::to_string(scope)?,job.created_at_ms],
        )?;
        let id: i64 = transaction.query_row(
            "SELECT id FROM memory_extraction_jobs WHERE idempotency_key=?1",
            [&job.idempotency_key],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO memory_extraction_events(job_id,event_id,kind,content) VALUES(?1,?2,?3,?4)",
            params![id, event.event_id, event.kind, event.content],
        )?;
        transaction.commit()?;
        drop(connection);
        if inserted == 1 {
            self.refresh_job_metrics()?;
        }
        Ok(id)
    }

    /// Mark the currently queued extraction batch for one explicit Session or Goal
    /// lifecycle as complete. Recording an event alone must never imply completion.
    pub fn complete_scope(
        &self,
        scope: &MemoryScope,
        completed_at_ms: u64,
    ) -> anyhow::Result<usize> {
        anyhow::ensure!(
            matches!(scope, MemoryScope::Session(_) | MemoryScope::Goal(_)),
            "only Session or Goal extraction scopes can be completed"
        );
        let scope_json = serde_json::to_string(scope)?;
        let connection = self.connection.lock().unwrap();
        let changed = connection.execute(
            "UPDATE memory_extraction_jobs SET completed_at_ms=?1,updated_at_ms=?1 WHERE scope_json=?2 AND completed_at_ms IS NULL AND status='pending'",
            params![completed_at_ms, scope_json],
        )?;
        drop(connection);
        if changed > 0 {
            self.refresh_job_metrics()?;
        }
        Ok(changed)
    }
    pub fn claim_extraction(
        &self,
        owner: &str,
        now_ms: u64,
        lease_ms: u64,
        max_age_ms: u64,
    ) -> anyhow::Result<Option<LeasedExtraction>> {
        anyhow::ensure!(
            !owner.trim().is_empty() && lease_ms > 0,
            "invalid extraction lease"
        );
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<ExtractionClaimRow> = tx.query_row("SELECT id,idempotency_key,session_id,goal_id,watermark,scope_json,attempts FROM memory_extraction_jobs WHERE ephemeral=0 AND memory_worker=0 AND completed_at_ms IS NOT NULL AND completed_at_ms<=?1 AND completed_at_ms>=?2 AND ((status IN ('pending','retryable_failure') AND retry_at_ms<=?1) OR (status='leased' AND lease_until_ms<=?1)) ORDER BY completed_at_ms,id LIMIT 1",params![now_ms,now_ms.saturating_sub(max_age_ms)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let Some((id, key, session, goal, watermark, scope_json, attempts)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        let until = now_ms.saturating_add(lease_ms);
        let changed=tx.execute("UPDATE memory_extraction_jobs SET status='leased',lease_owner=?1,lease_until_ms=?2,attempts=attempts+1,updated_at_ms=?3 WHERE id=?4 AND (status!='leased' OR lease_until_ms<=?3)",params![owner,until,now_ms,id])?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        tx.commit()?;
        drop(c);
        self.refresh_job_metrics()?;
        let scope = match scope_json {
            Some(value) => serde_json::from_str(&value)?,
            None => goal
                .as_ref()
                .map(|value| MemoryScope::Goal(value.clone()))
                .unwrap_or_else(|| MemoryScope::Session(session.clone())),
        };
        Ok(Some(LeasedExtraction {
            id,
            idempotency_key: key,
            session_id: session,
            goal_id: goal,
            watermark,
            scope,
            attempts: attempts + 1,
            lease_owner: owner.into(),
            lease_until_ms: until,
        }))
    }

    pub fn extraction_events(
        &self,
        lease: &LeasedExtraction,
        limit: usize,
    ) -> anyhow::Result<Vec<CanonicalMemoryEvent>> {
        anyhow::ensure!(limit > 0, "event limit must be positive");
        let connection = self.connection.lock().unwrap();
        let mut query = connection.prepare(
            "SELECT event_id,kind,content FROM memory_extraction_events WHERE job_id=?1 ORDER BY event_id LIMIT ?2",
        )?;
        let rows = query
            .query_map(params![lease.id, limit], |row| {
                Ok(CanonicalMemoryEvent {
                    event_id: row.get(0)?,
                    kind: row.get(1)?,
                    content: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn pending_scopes(&self, limit: usize) -> anyhow::Result<Vec<MemoryScope>> {
        let connection = self.connection.lock().unwrap();
        let mut query = connection.prepare(
            "SELECT DISTINCT scope_json FROM memory_candidates WHERE decision IS NULL ORDER BY scope_json LIMIT ?1",
        )?;
        let rows = query
            .query_map([limit], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .collect()
    }

    pub fn consolidated_record_count(&self) -> anyhow::Result<usize> {
        Ok(self.connection.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM memory_records",
            [],
            |row| row.get(0),
        )?)
    }
    pub fn complete(
        &self,
        lease: &LeasedExtraction,
        completion: ExtractionCompletion,
        now_ms: u64,
    ) -> anyhow::Result<()> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String, String)> = tx
            .query_row(
                "SELECT status,COALESCE(lease_owner,'') FROM memory_extraction_jobs WHERE id=?1",
                [lease.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match current.as_ref().map(|v| v.0.as_str()) {
            Some("succeeded" | "succeeded_no_output" | "permanent_failure") => {
                tx.commit()?;
                return Ok(());
            }
            Some("leased") if current.as_ref().unwrap().1 == lease.lease_owner => {}
            _ => anyhow::bail!("extraction lease is not owned"),
        }
        let (status, error, retry, candidates) = match completion {
            ExtractionCompletion::Succeeded { candidates } => {
                (ExtractionStatus::Succeeded, None, 0, candidates)
            }
            ExtractionCompletion::SucceededNoOutput => {
                (ExtractionStatus::SucceededNoOutput, None, 0, vec![])
            }
            ExtractionCompletion::RetryableFailure { error, retry_at_ms } => (
                ExtractionStatus::RetryableFailure,
                Some(error),
                retry_at_ms,
                vec![],
            ),
            ExtractionCompletion::PermanentFailure { error } => {
                (ExtractionStatus::PermanentFailure, Some(error), 0, vec![])
            }
        };
        for candidate in candidates {
            let key = format!("{}:{}", lease.id, candidate.content_hash);
            tx.execute("INSERT OR IGNORE INTO memory_candidates(job_id,candidate_key,kind_json,claim,source_event_ids_json,confidence,scope_json,valid_from_ms,valid_until_ms,redaction_version,content_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![lease.id,key,serde_json::to_string(&candidate.kind)?,candidate.claim,serde_json::to_string(&candidate.source_event_ids)?,candidate.confidence,serde_json::to_string(&candidate.proposed_scope)?,candidate.valid_from_ms,candidate.valid_until_ms,candidate.redaction_version,candidate.content_hash])?;
        }
        tx.execute("UPDATE memory_extraction_jobs SET status=?1,last_error=?2,retry_at_ms=?3,lease_owner=NULL,lease_until_ms=NULL,updated_at_ms=?4 WHERE id=?5",params![status.as_str(),error,retry,now_ms,lease.id])?;
        tx.commit()?;
        drop(c);
        self.refresh_job_metrics()?;
        Ok(())
    }
    pub fn status(&self, key: &str) -> anyhow::Result<ExtractionStatus> {
        let c = self.connection.lock().unwrap();
        let s: String = c.query_row(
            "SELECT status FROM memory_extraction_jobs WHERE idempotency_key=?1",
            [key],
            |r| r.get(0),
        )?;
        parse_status(&s)
    }
    pub fn acquire_scope(
        &self,
        scope: &MemoryScope,
        owner: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> anyhow::Result<Option<ScopeLease>> {
        scope.validate()?;
        let key = serde_json::to_string(scope)?;
        let c = self.connection.lock().unwrap();
        let until = now_ms.saturating_add(lease_ms);
        let n=c.execute("INSERT INTO memory_scope_leases(scope_key,owner,lease_until_ms) VALUES(?1,?2,?3) ON CONFLICT(scope_key) DO UPDATE SET owner=excluded.owner,lease_until_ms=excluded.lease_until_ms WHERE memory_scope_leases.lease_until_ms<=?4 OR memory_scope_leases.owner=?2",params![key,owner,until,now_ms])?;
        Ok((n == 1).then(|| ScopeLease {
            scope: scope.clone(),
            owner: owner.into(),
            lease_until_ms: until,
        }))
    }
    pub fn pending_candidates(
        &self,
        scope: &MemoryScope,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, MemoryCandidate)>> {
        let key = serde_json::to_string(scope)?;
        let c = self.connection.lock().unwrap();
        let mut q=c.prepare("SELECT id,kind_json,claim,source_event_ids_json,confidence,scope_json,valid_from_ms,valid_until_ms,redaction_version,content_hash FROM memory_candidates WHERE scope_json=?1 AND decision IS NULL ORDER BY content_hash,id LIMIT ?2")?;
        let rows = q.query_map(params![key, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, String>(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get::<_, u32>(8)?,
                r.get::<_, String>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, k, claim, events, confidence, s, vf, vu, rv, hash) = row?;
            Ok((
                id,
                MemoryCandidate {
                    kind: serde_json::from_str(&k)?,
                    claim,
                    source_event_ids: serde_json::from_str(&events)?,
                    confidence,
                    proposed_scope: serde_json::from_str(&s)?,
                    valid_from_ms: vf,
                    valid_until_ms: vu,
                    redaction_version: rv,
                    content_hash: hash,
                },
            ))
        })
        .collect()
    }

    pub(crate) fn current_records(
        &self,
        scope: &MemoryScope,
    ) -> anyhow::Result<Vec<ConsolidatedRecord>> {
        let scope_json = serde_json::to_string(scope)?;
        let connection = self.connection.lock().unwrap();
        let mut query = connection.prepare(
            "SELECT record_id,kind_json,content,source_event_ids_json,content_hash FROM memory_records WHERE scope_json=?1 AND status='current' ORDER BY record_id",
        )?;
        let records = query
            .query_map([scope_json], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .map(|row| {
                let (id, kind, content, source_event_ids, content_hash) = row?;
                Ok(ConsolidatedRecord {
                    id: MemoryRecordId(id),
                    kind: serde_json::from_str(&kind)?,
                    content,
                    source_event_ids: serde_json::from_str(&source_event_ids)?,
                    content_hash,
                })
            })
            .collect();
        records
    }
    pub fn commit_decisions(
        &self,
        lease: &ScopeLease,
        watermark: &str,
        decisions: &[(i64, String, Option<String>)],
        now_ms: u64,
    ) -> anyhow::Result<()> {
        let key = serde_json::to_string(&lease.scope)?;
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owned:bool=tx.query_row("SELECT COUNT(*)=1 FROM memory_scope_leases WHERE scope_key=?1 AND owner=?2 AND lease_until_ms>=?3",params![key,lease.owner,now_ms],|r|r.get(0))?;
        anyhow::ensure!(owned, "scope lease is not owned");
        for (id, d, r) in decisions {
            tx.execute("UPDATE memory_candidates SET decision=?1,decided_record_id=?2 WHERE id=?3 AND decision IS NULL",params![d,r,id])?;
        }
        let candidate_snapshot = decisions
            .iter()
            .map(|(id, _, _)| {
                tx.query_row(
                    "SELECT kind_json,claim,source_event_ids_json,scope_json,content_hash FROM memory_candidates WHERE id=?1",
                    [id],
                    |row| {
                        Ok(serde_json::json!({
                            "candidate_id": id,
                            "kind": row.get::<_, String>(0)?,
                            "claim": row.get::<_, String>(1)?,
                            "source_event_ids": row.get::<_, String>(2)?,
                            "scope": row.get::<_, String>(3)?,
                            "content_hash": row.get::<_, String>(4)?,
                        }))
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (id, decision, record_id) in decisions {
            if let Some(record_id) = record_id {
                if decision == "\"supersede\"" {
                    tx.execute(
                        "UPDATE memory_records SET status='superseded' WHERE status='current' AND (kind_json,scope_json,source_event_ids_json)=(SELECT kind_json,scope_json,source_event_ids_json FROM memory_candidates WHERE id=?1) AND content_hash<>(SELECT content_hash FROM memory_candidates WHERE id=?1)",
                        [id],
                    )?;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO memory_records(record_id,candidate_id,scope_json,kind_json,content,source_event_ids_json,content_hash,status,version,created_at_ms) SELECT ?1,c.id,c.scope_json,c.kind_json,c.claim,c.source_event_ids_json,c.content_hash,'current',COALESCE((SELECT MAX(r.version)+1 FROM memory_records r WHERE r.kind_json=c.kind_json AND r.scope_json=c.scope_json AND r.source_event_ids_json=c.source_event_ids_json),1),?2 FROM memory_candidates c WHERE c.id=?3 AND ?4 IN ('\"insert\"','\"supersede\"')",
                    params![record_id, now_ms, id, decision],
                )?;
            }
        }
        let snapshot = serde_json::to_string(&candidate_snapshot)?;
        let decisions_json = serde_json::to_string(decisions)?;
        tx.execute(
            "INSERT OR IGNORE INTO memory_consolidation_runs(scope_key,owner,candidate_snapshot_json,watermark,decisions_json,completed_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
            params![key, lease.owner, snapshot, watermark, decisions_json, now_ms],
        )?;
        tx.execute(
            "DELETE FROM memory_scope_leases WHERE scope_key=?1 AND owner=?2",
            params![key, lease.owner],
        )?;
        tx.commit()?;
        Ok(())
    }
}
fn parse_status(s: &str) -> anyhow::Result<ExtractionStatus> {
    Ok(match s {
        "pending" => ExtractionStatus::Pending,
        "leased" => ExtractionStatus::Leased,
        "succeeded" => ExtractionStatus::Succeeded,
        "succeeded_no_output" => ExtractionStatus::SucceededNoOutput,
        "retryable_failure" => ExtractionStatus::RetryableFailure,
        "permanent_failure" => ExtractionStatus::PermanentFailure,
        _ => anyhow::bail!("unknown extraction status"),
    })
}
