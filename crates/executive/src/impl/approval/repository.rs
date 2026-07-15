use crate::r#impl::goal::migrations;
use fabric::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalContractError, ApprovalId, ApprovalResolution,
    ApprovalRisk, ApprovalSnapshot, ApprovalStatus, ApprovalSubject, AttemptId, CodingJobId,
    GoalId, OperationId, PrincipalId,
};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

const MAX_SUMMARY_BYTES: usize = 16 * 1024;
const MAX_ARTIFACTS_JSON_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct ApprovalCreate {
    pub subject: ApprovalSubject,
    pub risk: ApprovalRisk,
    pub summary: String,
    pub artifacts: Vec<ApprovalArtifactRef>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalResolutionContext {
    pub principal_id: PrincipalId,
    pub channel: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Reject { reason: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDelivery {
    pub approval_id: ApprovalId,
    pub channel: String,
    pub conversation_id: String,
    pub correlation_id: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ApprovalApplyReceipt {
    pub approval_id: ApprovalId,
    pub operation_id: OperationId,
    pub goal_id: GoalId,
    pub success: bool,
    pub applied_head: Option<String>,
    pub diff_sha256: String,
    pub changed_paths: Vec<std::path::PathBuf>,
    pub error: Option<String>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalApplyOperation {
    pub approval_id: ApprovalId,
    pub operation_id: OperationId,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalApplyClaim {
    Claimed(ApprovalApplyOperation),
    Existing(ApprovalApplyOperation),
}

#[derive(Debug, Clone)]
pub struct ApprovalChannelPolicy {
    allowed: BTreeSet<String>,
}
impl ApprovalChannelPolicy {
    pub fn new(channels: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed: channels.into_iter().collect(),
        }
    }
    pub fn permits(&self, channel: &str) -> bool {
        self.allowed.contains(channel)
    }
}
impl Default for ApprovalChannelPolicy {
    fn default() -> Self {
        Self::new(["telegram".into(), "local_rpc".into()])
    }
}

pub struct ApprovalRepository {
    db: Connection,
    channel_policy: ApprovalChannelPolicy,
}

impl ApprovalRepository {
    pub fn open(path: &Path) -> Result<Self, ApprovalRepositoryError> {
        let db = Connection::open(path)?;
        migrations::run_migrations(&db)
            .map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))?;
        Ok(Self {
            db,
            channel_policy: ApprovalChannelPolicy::default(),
        })
    }

    pub fn with_channel_policy(mut self, policy: ApprovalChannelPolicy) -> Self {
        self.channel_policy = policy;
        self
    }

    pub fn claim_apply(
        &self,
        approval_id: ApprovalId,
        operation_id: OperationId,
        now_ms: i64,
    ) -> Result<ApprovalApplyClaim, ApprovalRepositoryError> {
        let approval = self
            .get(approval_id)?
            .ok_or(ApprovalRepositoryError::NotFound(approval_id))?;
        if approval.status != ApprovalStatus::Approved {
            return Err(ApprovalRepositoryError::NotApproved);
        }
        let changed = self.db.execute(
            "INSERT OR IGNORE INTO approval_apply_operations (
                approval_id, operation_id, status, started_at_ms
             ) VALUES (?1,?2,'running',?3)",
            params![
                approval_id.0.to_string(),
                operation_id.0.to_string(),
                now_ms
            ],
        )?;
        let operation = self
            .apply_operation(approval_id)?
            .ok_or_else(|| ApprovalRepositoryError::Storage("apply claim disappeared".into()))?;
        Ok(if changed == 1 {
            ApprovalApplyClaim::Claimed(operation)
        } else {
            ApprovalApplyClaim::Existing(operation)
        })
    }

    pub fn apply_operation(
        &self,
        approval_id: ApprovalId,
    ) -> Result<Option<ApprovalApplyOperation>, ApprovalRepositoryError> {
        self.db
            .query_row(
                "SELECT operation_id,status,started_at_ms,finished_at_ms,error
                 FROM approval_apply_operations WHERE approval_id=?1",
                params![approval_id.0.to_string()],
                |row| {
                    let operation: String = row.get(0)?;
                    let operation = uuid::Uuid::parse_str(&operation).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            error.into(),
                        )
                    })?;
                    Ok(ApprovalApplyOperation {
                        approval_id,
                        operation_id: OperationId(operation),
                        status: row.get(1)?,
                        started_at_ms: row.get(2)?,
                        finished_at_ms: row.get(3)?,
                        error: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn apply_receipt(
        &self,
        approval_id: ApprovalId,
    ) -> Result<Option<ApprovalApplyReceipt>, ApprovalRepositoryError> {
        self.db
            .query_row(
                "SELECT receipt_json FROM approval_apply_receipts WHERE approval_id=?1",
                params![approval_id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|json| from_wire(&json))
            .transpose()
    }

    pub fn finish_apply(
        &self,
        receipt: &ApprovalApplyReceipt,
    ) -> Result<ApprovalSnapshot, ApprovalRepositoryError> {
        if let Some(existing) = self.apply_receipt(receipt.approval_id)? {
            if existing == *receipt {
                return self
                    .get(receipt.approval_id)?
                    .ok_or(ApprovalRepositoryError::NotFound(receipt.approval_id));
            }
            return Err(ApprovalRepositoryError::AlreadyDecided);
        }
        let operation = self
            .apply_operation(receipt.approval_id)?
            .ok_or(ApprovalRepositoryError::ApplyNotClaimed)?;
        if operation.operation_id != receipt.operation_id || operation.status != "running" {
            return Err(ApprovalRepositoryError::AlreadyDecided);
        }
        let current = self
            .get(receipt.approval_id)?
            .ok_or(ApprovalRepositoryError::NotFound(receipt.approval_id))?;
        let next = current.consume(current.version)?;
        let tx = self.db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE approval_requests SET status='consumed',version=?1
             WHERE approval_id=?2 AND version=?3 AND status='approved'",
            params![
                next.version,
                receipt.approval_id.0.to_string(),
                current.version
            ],
        )?;
        if changed != 1 {
            return Err(ApprovalRepositoryError::VersionConflict {
                expected: current.version,
                actual: self
                    .get(receipt.approval_id)?
                    .map(|v| v.version)
                    .unwrap_or(0),
            });
        }
        tx.execute(
            "UPDATE approval_apply_operations SET status=?1,finished_at_ms=?2,error=?3
             WHERE approval_id=?4 AND operation_id=?5 AND status='running'",
            params![
                if receipt.success {
                    "succeeded"
                } else {
                    "failed"
                },
                receipt.finished_at_ms,
                receipt.error,
                receipt.approval_id.0.to_string(),
                receipt.operation_id.0.to_string(),
            ],
        )?;
        tx.execute(
            "INSERT INTO approval_apply_receipts
             (approval_id,operation_id,receipt_json,created_at_ms) VALUES (?1,?2,?3,?4)",
            params![
                receipt.approval_id.0.to_string(),
                receipt.operation_id.0.to_string(),
                wire(receipt)?,
                receipt.finished_at_ms,
            ],
        )?;
        append_event(
            &tx,
            receipt.approval_id,
            next.version,
            "consumed",
            &serde_json::json!({"operation_id":receipt.operation_id.0,"success":receipt.success}),
            receipt.finished_at_ms,
        )?;
        tx.commit()?;
        self.get(receipt.approval_id)?
            .ok_or(ApprovalRepositoryError::NotFound(receipt.approval_id))
    }

    pub fn record_delivery_pending(
        &self,
        approval_id: ApprovalId,
        channel: &str,
        conversation_id: &str,
        correlation_id: &str,
        now_ms: i64,
    ) -> Result<ApprovalDelivery, ApprovalRepositoryError> {
        let approval = self
            .get(approval_id)?
            .ok_or(ApprovalRepositoryError::NotFound(approval_id))?;
        if approval.status != ApprovalStatus::Pending {
            return Err(ApprovalRepositoryError::AlreadyDecided);
        }
        self.db.execute(
            "INSERT INTO approval_deliveries (
                approval_id, channel, conversation_id, correlation_id, status,
                created_at_ms, updated_at_ms
             ) VALUES (?1,?2,?3,?4,'pending',?5,?5)
             ON CONFLICT(approval_id, channel) DO NOTHING",
            params![
                approval_id.0.to_string(),
                channel,
                conversation_id,
                correlation_id,
                now_ms
            ],
        )?;
        self.delivery_for_correlation(correlation_id)?
            .ok_or_else(|| ApprovalRepositoryError::Storage("approval delivery disappeared".into()))
    }

    pub fn record_delivery_sent(
        &self,
        correlation_id: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> Result<(), ApprovalRepositoryError> {
        let changed = self.db.execute(
            "UPDATE approval_deliveries SET status='sent', provider_message_id=?1,
             attempt_count=attempt_count+1, last_error=NULL, updated_at_ms=?2
             WHERE correlation_id=?3",
            params![provider_message_id, now_ms, correlation_id],
        )?;
        if changed != 1 {
            return Err(ApprovalRepositoryError::Storage(
                "approval delivery correlation not found".into(),
            ));
        }
        Ok(())
    }

    pub fn record_delivery_failed(
        &self,
        correlation_id: &str,
        error: &str,
        now_ms: i64,
    ) -> Result<(), ApprovalRepositoryError> {
        let changed = self.db.execute(
            "UPDATE approval_deliveries SET status='failed', attempt_count=attempt_count+1,
             last_error=?1, updated_at_ms=?2 WHERE correlation_id=?3",
            params![bound_text(error, 1024), now_ms, correlation_id],
        )?;
        if changed != 1 {
            return Err(ApprovalRepositoryError::Storage(
                "approval delivery correlation not found".into(),
            ));
        }
        Ok(())
    }

    pub fn delivery_for_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<Option<ApprovalDelivery>, ApprovalRepositoryError> {
        self.db
            .query_row(
                "SELECT approval_id, channel, conversation_id, correlation_id, status,
                        provider_message_id, attempt_count, last_error, created_at_ms, updated_at_ms
                 FROM approval_deliveries WHERE correlation_id=?1",
                params![correlation_id],
                |row| {
                    let id: String = row.get(0)?;
                    let id = uuid::Uuid::parse_str(&id).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            error.into(),
                        )
                    })?;
                    Ok(ApprovalDelivery {
                        approval_id: ApprovalId(id),
                        channel: row.get(1)?,
                        conversation_id: row.get(2)?,
                        correlation_id: row.get(3)?,
                        status: row.get(4)?,
                        provider_message_id: row.get(5)?,
                        attempt_count: row.get::<_, i64>(6)? as u32,
                        last_error: row.get(7)?,
                        created_at_ms: row.get(8)?,
                        updated_at_ms: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn create(
        &self,
        create: ApprovalCreate,
    ) -> Result<ApprovalSnapshot, ApprovalRepositoryError> {
        validate_create(&create)?;
        let subject = create.subject.canonicalized()?;
        let subject_hash = subject.subject_hash()?;
        let category = wire(&subject.category)?;
        if let Some(existing) = self.active_for_subject(subject.category, &subject_hash)? {
            if existing.subject == subject
                && existing.risk == create.risk
                && existing.summary == create.summary
                && existing.artifacts == create.artifacts
            {
                return Ok(existing);
            }
            return Err(ApprovalRepositoryError::ActiveSubjectConflict);
        }
        let owner_id = self.owner_for_goal(subject.goal_id)?;
        self.validate_subject_refs(&subject)?;
        let id = ApprovalId::new();
        let subject_json = bounded_json(&subject, MAX_ARTIFACTS_JSON_BYTES, "approval subject")?;
        let artifacts_json = bounded_json(
            &create.artifacts,
            MAX_ARTIFACTS_JSON_BYTES,
            "approval artifacts",
        )?;
        let risk = wire(&create.risk)?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO approval_requests (
                approval_id, objective_id, attempt_id, job_id, owner_id, category, risk,
                subject_json, subject_hash, summary, artifacts_json, created_at_ms,
                expires_at_ms, status, version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'pending', 0)",
            params![
                id.0.to_string(),
                subject.goal_id.0,
                subject.attempt_id.map(|value| value.0.to_string()),
                subject.job_id.map(|value| value.0.to_string()),
                owner_id.0,
                category,
                risk,
                subject_json,
                subject_hash,
                create.summary,
                artifacts_json,
                create.created_at_ms,
                create.expires_at_ms
            ],
        )?;
        append_event(
            &tx,
            id,
            0,
            "created",
            &serde_json::json!({"status":"pending"}),
            create.created_at_ms,
        )?;
        tx.commit()?;
        self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))
    }

    pub fn get(&self, id: ApprovalId) -> Result<Option<ApprovalSnapshot>, ApprovalRepositoryError> {
        self.db
            .query_row(
                &format!("SELECT {APPROVAL_COLS} FROM approval_requests WHERE approval_id = ?1"),
                params![id.0.to_string()],
                map_snapshot,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_pending(
        &self,
        owner: &PrincipalId,
        now_ms: i64,
    ) -> Result<Vec<ApprovalSnapshot>, ApprovalRepositoryError> {
        self.expire(now_ms)?;
        let mut statement = self.db.prepare(&format!(
            "SELECT {APPROVAL_COLS} FROM approval_requests
             WHERE owner_id = ?1 AND status = 'pending' ORDER BY created_at_ms, approval_id"
        ))?;
        let pending = statement
            .query_map(params![owner.0], map_snapshot)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into);
        pending
    }

    pub fn resolve(
        &self,
        id: ApprovalId,
        expected_version: u64,
        context: &ApprovalResolutionContext,
        decision: ApprovalDecision,
        now_ms: i64,
    ) -> Result<ApprovalSnapshot, ApprovalRepositoryError> {
        let current = self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))?;
        if current.status != ApprovalStatus::Pending {
            if resolution_matches(&current, context, &decision) {
                return Ok(current);
            }
            return Err(ApprovalRepositoryError::AlreadyDecided);
        }
        if current.version != expected_version {
            return Err(ApprovalRepositoryError::VersionConflict {
                expected: expected_version,
                actual: current.version,
            });
        }
        if current.owner_id != context.principal_id {
            return Err(ApprovalRepositoryError::WrongOwner);
        }
        if !self.channel_policy.permits(&context.channel) {
            return Err(ApprovalRepositoryError::ChannelDenied);
        }
        let resolution = if now_ms >= current.expires_at_ms {
            ApprovalResolution::expired(now_ms)
        } else {
            match decision {
                ApprovalDecision::Approve => ApprovalResolution::approved(
                    context.principal_id.clone(),
                    context.channel.clone(),
                    now_ms,
                ),
                ApprovalDecision::Reject { reason } => ApprovalResolution::rejected(
                    context.principal_id.clone(),
                    context.channel.clone(),
                    now_ms,
                    reason,
                ),
            }
        };
        let next = current.resolve(expected_version, resolution.clone())?;
        self.persist_resolution(&current, &next, &resolution)?;
        self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))
    }

    pub fn expire(&self, now_ms: i64) -> Result<Vec<ApprovalSnapshot>, ApprovalRepositoryError> {
        let ids: Vec<_> = {
            let mut statement = self.db.prepare(
                "SELECT approval_id FROM approval_requests WHERE status = 'pending' AND expires_at_ms <= ?1 ORDER BY approval_id"
            )?;
            let ids = statement
                .query_map(params![now_ms], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids
        };
        let mut expired = Vec::with_capacity(ids.len());
        for id in ids {
            let id = ApprovalId(
                uuid::Uuid::parse_str(&id)
                    .map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))?,
            );
            let current = self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))?;
            let resolution = ApprovalResolution::expired(now_ms);
            let next = current.resolve(current.version, resolution.clone())?;
            self.persist_resolution(&current, &next, &resolution)?;
            expired.push(next);
        }
        Ok(expired)
    }

    pub fn deny_delivery_failure(
        &self,
        id: ApprovalId,
        expected_version: u64,
        now_ms: i64,
        reason: &str,
    ) -> Result<ApprovalSnapshot, ApprovalRepositoryError> {
        let current = self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))?;
        if current.version != expected_version {
            return Err(ApprovalRepositoryError::VersionConflict {
                expected: expected_version,
                actual: current.version,
            });
        }
        if current.status != ApprovalStatus::Pending {
            return Err(ApprovalRepositoryError::AlreadyDecided);
        }
        let mut resolution = ApprovalResolution::expired(now_ms);
        resolution.reason = Some(format!("approval delivery failed and was denied: {reason}"));
        let next = current.resolve(expected_version, resolution.clone())?;
        self.persist_resolution(&current, &next, &resolution)?;
        self.get(id)?.ok_or(ApprovalRepositoryError::NotFound(id))
    }

    fn persist_resolution(
        &self,
        current: &ApprovalSnapshot,
        next: &ApprovalSnapshot,
        resolution: &ApprovalResolution,
    ) -> Result<(), ApprovalRepositoryError> {
        let tx = self.db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE approval_requests SET status=?1, version=?2, resolution_principal=?3,
             resolution_channel=?4, resolution_time_ms=?5, resolution_reason=?6
             WHERE approval_id=?7 AND version=?8 AND status='pending'",
            params![
                status_wire(next.status),
                next.version,
                resolution.principal_id.as_ref().map(|id| id.0.as_str()),
                resolution.channel,
                resolution.resolved_at_ms,
                resolution.reason,
                current.id.0.to_string(),
                current.version
            ],
        )?;
        if changed != 1 {
            return Err(ApprovalRepositoryError::VersionConflict {
                expected: current.version,
                actual: self
                    .get(current.id)?
                    .map(|value| value.version)
                    .unwrap_or(current.version),
            });
        }
        append_event(
            &tx,
            current.id,
            next.version,
            "resolved",
            &serde_json::json!({"status": status_wire(next.status)}),
            resolution.resolved_at_ms,
        )?;
        tx.commit()?;
        Ok(())
    }

    fn active_for_subject(
        &self,
        category: ApprovalCategory,
        subject_hash: &str,
    ) -> Result<Option<ApprovalSnapshot>, ApprovalRepositoryError> {
        self.db.query_row(&format!(
            "SELECT {APPROVAL_COLS} FROM approval_requests WHERE category=?1 AND subject_hash=?2 AND status IN ('pending','approved')"
        ), params![wire(&category)?, subject_hash], map_snapshot).optional().map_err(Into::into)
    }

    fn owner_for_goal(&self, goal_id: GoalId) -> Result<PrincipalId, ApprovalRepositoryError> {
        self.db
            .query_row(
                "SELECT owner_id FROM objectives WHERE objective_id=?1",
                params![goal_id.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PrincipalId)
            .ok_or(ApprovalRepositoryError::GoalNotFound(goal_id))
    }

    fn validate_subject_refs(
        &self,
        subject: &ApprovalSubject,
    ) -> Result<(), ApprovalRepositoryError> {
        if let Some(attempt_id) = subject.attempt_id {
            let goal: Option<i64> = self
                .db
                .query_row(
                    "SELECT objective_id FROM goal_attempts WHERE attempt_id=?1",
                    params![attempt_id.0.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if goal != Some(subject.goal_id.0) {
                return Err(ApprovalRepositoryError::ReferenceMismatch);
            }
        }
        if let Some(job_id) = subject.job_id {
            let identity: Option<(i64, String)> = self
                .db
                .query_row(
                    "SELECT objective_id, attempt_id FROM goal_coding_jobs WHERE job_id=?1",
                    params![job_id.0.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if identity.as_ref().map(|value| value.0) != Some(subject.goal_id.0)
                || subject.attempt_id.map(|id| id.0.to_string()) != identity.map(|value| value.1)
            {
                return Err(ApprovalRepositoryError::ReferenceMismatch);
            }
        }
        Ok(())
    }
}

const APPROVAL_COLS: &str =
    "approval_id, objective_id, attempt_id, job_id, owner_id, category, risk,
 subject_json, subject_hash, summary, artifacts_json, created_at_ms, expires_at_ms, status, version,
 resolution_principal, resolution_channel, resolution_time_ms, resolution_reason";

fn map_snapshot(row: &Row<'_>) -> rusqlite::Result<ApprovalSnapshot> {
    let parse_error = |index, error: String| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, error.into())
    };
    let id: String = row.get(0)?;
    let attempt: Option<String> = row.get(2)?;
    let job: Option<String> = row.get(3)?;
    let category: String = row.get(5)?;
    let risk: String = row.get(6)?;
    let subject_json: String = row.get(7)?;
    let artifacts_json: String = row.get(10)?;
    let status_text: String = row.get(13)?;
    let status = parse_status(&status_text).map_err(|error| parse_error(13, error.to_string()))?;
    let resolution_principal: Option<String> = row.get(15)?;
    let resolution_channel: Option<String> = row.get(16)?;
    let resolution_time_ms: Option<i64> = row.get(17)?;
    let resolution_reason: Option<String> = row.get(18)?;
    let resolution = resolution_time_ms.map(|resolved_at_ms| ApprovalResolution {
        status: if status == ApprovalStatus::Consumed {
            ApprovalStatus::Approved
        } else {
            status
        },
        principal_id: resolution_principal.map(PrincipalId),
        channel: resolution_channel,
        resolved_at_ms,
        reason: resolution_reason,
    });
    let snapshot = ApprovalSnapshot {
        id: ApprovalId(
            uuid::Uuid::parse_str(&id).map_err(|error| parse_error(0, error.to_string()))?,
        ),
        goal_id: GoalId(row.get(1)?),
        attempt_id: attempt
            .map(|value| uuid::Uuid::parse_str(&value).map(AttemptId))
            .transpose()
            .map_err(|error| parse_error(2, error.to_string()))?,
        job_id: job
            .map(|value| uuid::Uuid::parse_str(&value).map(CodingJobId))
            .transpose()
            .map_err(|error| parse_error(3, error.to_string()))?,
        owner_id: PrincipalId(row.get(4)?),
        category: from_wire(&category).map_err(|error| parse_error(5, error.to_string()))?,
        risk: from_wire(&risk).map_err(|error| parse_error(6, error.to_string()))?,
        subject: serde_json::from_str(&subject_json)
            .map_err(|error| parse_error(7, error.to_string()))?,
        subject_hash: row.get(8)?,
        summary: row.get(9)?,
        artifacts: serde_json::from_str(&artifacts_json)
            .map_err(|error| parse_error(10, error.to_string()))?,
        created_at_ms: row.get(11)?,
        expires_at_ms: row.get(12)?,
        status,
        version: row.get::<_, i64>(14)? as u64,
        resolution,
    };
    snapshot
        .validate()
        .map_err(|error| parse_error(7, error.to_string()))?;
    Ok(snapshot)
}

fn validate_create(create: &ApprovalCreate) -> Result<(), ApprovalRepositoryError> {
    if create.summary.trim().is_empty()
        || create.summary.len() > MAX_SUMMARY_BYTES
        || create.expires_at_ms <= create.created_at_ms
    {
        return Err(ApprovalRepositoryError::InvalidRequest);
    }
    for artifact in &create.artifacts {
        artifact.validate()?;
    }
    Ok(())
}
fn append_event(
    tx: &Transaction<'_>,
    id: ApprovalId,
    version: u64,
    event_type: &str,
    payload: &serde_json::Value,
    now_ms: i64,
) -> Result<(), ApprovalRepositoryError> {
    tx.execute("INSERT INTO approval_events (approval_id, version, event_type, payload_json, created_at_ms) VALUES (?1,?2,?3,?4,?5)",
        params![id.0.to_string(), version, event_type, serde_json::to_string(payload).map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))?, now_ms])?;
    Ok(())
}
fn wire<T: Serialize>(value: &T) -> Result<String, ApprovalRepositoryError> {
    serde_json::to_string(value)
        .map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))
}
fn from_wire<T: DeserializeOwned>(value: &str) -> Result<T, ApprovalRepositoryError> {
    serde_json::from_str(value).map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))
}
fn bounded_json<T: Serialize>(
    value: &T,
    max: usize,
    label: &str,
) -> Result<String, ApprovalRepositoryError> {
    let json = serde_json::to_string(value)
        .map_err(|error| ApprovalRepositoryError::Storage(error.to_string()))?;
    if json.len() > max {
        return Err(ApprovalRepositoryError::Storage(format!(
            "{label} exceeds bounded size"
        )));
    }
    Ok(json)
}
fn bound_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}
fn status_wire(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Rejected => "rejected",
        ApprovalStatus::Expired => "expired",
        ApprovalStatus::Consumed => "consumed",
    }
}
fn parse_status(value: &str) -> Result<ApprovalStatus, ApprovalRepositoryError> {
    match value {
        "pending" => Ok(ApprovalStatus::Pending),
        "approved" => Ok(ApprovalStatus::Approved),
        "rejected" => Ok(ApprovalStatus::Rejected),
        "expired" => Ok(ApprovalStatus::Expired),
        "consumed" => Ok(ApprovalStatus::Consumed),
        _ => Err(ApprovalRepositoryError::Storage(
            "invalid approval status".into(),
        )),
    }
}
fn resolution_matches(
    snapshot: &ApprovalSnapshot,
    context: &ApprovalResolutionContext,
    decision: &ApprovalDecision,
) -> bool {
    let Some(resolution) = &snapshot.resolution else {
        return false;
    };
    let desired = match decision {
        ApprovalDecision::Approve => ApprovalStatus::Approved,
        ApprovalDecision::Reject { .. } => ApprovalStatus::Rejected,
    };
    resolution.status == desired
        && resolution.principal_id.as_ref() == Some(&context.principal_id)
        && resolution.channel.as_deref() == Some(context.channel.as_str())
}

#[derive(Debug)]
pub enum ApprovalRepositoryError {
    NotFound(ApprovalId),
    GoalNotFound(GoalId),
    WrongOwner,
    ChannelDenied,
    AlreadyDecided,
    ActiveSubjectConflict,
    ReferenceMismatch,
    InvalidRequest,
    NotApproved,
    ApplyNotClaimed,
    VersionConflict { expected: u64, actual: u64 },
    Contract(ApprovalContractError),
    Sql(rusqlite::Error),
    Storage(String),
}
impl From<rusqlite::Error> for ApprovalRepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}
impl From<ApprovalContractError> for ApprovalRepositoryError {
    fn from(value: ApprovalContractError) -> Self {
        Self::Contract(value)
    }
}
impl fmt::Display for ApprovalRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ApprovalRepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::goal::ObjectiveStore;
    use fabric::{GoalBudget, GoalSpec};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    struct Fixture {
        _file: NamedTempFile,
        repo: ApprovalRepository,
        goal_id: GoalId,
    }
    impl Fixture {
        fn new() -> Self {
            let file = NamedTempFile::new().unwrap();
            let store = ObjectiveStore::open(file.path()).unwrap();
            let goal = store
                .create_goal(
                    &PrincipalId("owner".into()),
                    "s",
                    "project",
                    &GoalSpec {
                        original_intent: "approve".into(),
                        desired_state: vec![],
                        constraints: vec![],
                        acceptance_criteria: vec![],
                        budget: GoalBudget {
                            max_input_tokens: 1,
                            max_output_tokens: 1,
                            max_cost_usd: None,
                            max_attempts: 1,
                            deadline_ms: None,
                        },
                    },
                )
                .unwrap();
            drop(store);
            let repo = ApprovalRepository::open(file.path()).unwrap();
            Self {
                _file: file,
                repo,
                goal_id: goal.id,
            }
        }
        fn create(&self) -> ApprovalCreate {
            ApprovalCreate {
                subject: ApprovalSubject {
                    category: ApprovalCategory::BudgetExpansion,
                    goal_id: self.goal_id,
                    attempt_id: None,
                    job_id: None,
                    attributes: BTreeMap::from([("limit".into(), "2".into())]),
                    allowed_scope: vec![PathBuf::from("project")],
                    apply_target: None,
                },
                risk: ApprovalRisk::High,
                summary: "Increase bounded budget".into(),
                artifacts: vec![],
                created_at_ms: 100,
                expires_at_ms: 200,
            }
        }
    }

    #[test]
    fn duplicate_creation_is_idempotent_and_restart_safe() {
        let f = Fixture::new();
        let first = f.repo.create(f.create()).unwrap();
        let second = f.repo.create(f.create()).unwrap();
        assert_eq!(first.id, second.id);
        let reopened = ApprovalRepository::open(f._file.path()).unwrap();
        assert_eq!(reopened.get(first.id).unwrap().unwrap(), first);
    }
    #[test]
    fn approve_reject_replay_owner_channel_and_stale_versions() {
        let f = Fixture::new();
        let approval = f.repo.create(f.create()).unwrap();
        let wrong = ApprovalResolutionContext {
            principal_id: PrincipalId("attacker".into()),
            channel: "telegram".into(),
        };
        assert!(matches!(
            f.repo
                .resolve(approval.id, 0, &wrong, ApprovalDecision::Approve, 150),
            Err(ApprovalRepositoryError::WrongOwner)
        ));
        let owner = ApprovalResolutionContext {
            principal_id: PrincipalId("owner".into()),
            channel: "email".into(),
        };
        assert!(matches!(
            f.repo
                .resolve(approval.id, 0, &owner, ApprovalDecision::Approve, 150),
            Err(ApprovalRepositoryError::ChannelDenied)
        ));
        let owner = ApprovalResolutionContext {
            principal_id: PrincipalId("owner".into()),
            channel: "telegram".into(),
        };
        assert!(matches!(
            f.repo
                .resolve(approval.id, 9, &owner, ApprovalDecision::Approve, 150),
            Err(ApprovalRepositoryError::VersionConflict { .. })
        ));
        let approved = f
            .repo
            .resolve(approval.id, 0, &owner, ApprovalDecision::Approve, 150)
            .unwrap();
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(
            f.repo
                .resolve(approval.id, 0, &owner, ApprovalDecision::Approve, 150)
                .unwrap()
                .id,
            approval.id
        );
        assert!(matches!(
            f.repo.resolve(
                approval.id,
                1,
                &owner,
                ApprovalDecision::Reject { reason: None },
                160
            ),
            Err(ApprovalRepositoryError::AlreadyDecided)
        ));
        let mut second = f.create();
        second.subject.attributes.insert("limit".into(), "3".into());
        let second = f.repo.create(second).unwrap();
        let rejected = f
            .repo
            .resolve(
                second.id,
                0,
                &owner,
                ApprovalDecision::Reject {
                    reason: Some("no".into()),
                },
                150,
            )
            .unwrap();
        assert_eq!(rejected.status, ApprovalStatus::Rejected);
    }
    #[test]
    fn expiry_and_delivery_failure_are_denials() {
        let f = Fixture::new();
        let approval = f.repo.create(f.create()).unwrap();
        let pending = f
            .repo
            .list_pending(&PrincipalId("owner".into()), 200)
            .unwrap();
        assert!(pending.is_empty());
        assert_eq!(
            f.repo.get(approval.id).unwrap().unwrap().status,
            ApprovalStatus::Expired
        );
        let mut next = f.create();
        next.subject.attributes.insert("limit".into(), "4".into());
        let next = f.repo.create(next).unwrap();
        assert_eq!(
            f.repo
                .deny_delivery_failure(next.id, 0, 150, "provider unavailable")
                .unwrap()
                .status,
            ApprovalStatus::Expired
        );
    }
    #[test]
    fn event_failure_rolls_back_request_transaction() {
        let f = Fixture::new();
        f.repo.db.execute_batch("CREATE TRIGGER fail_approval_event BEFORE INSERT ON approval_events BEGIN SELECT RAISE(ABORT, 'event fail'); END;").unwrap();
        assert!(f.repo.create(f.create()).is_err());
        let count: i64 = f
            .repo
            .db
            .query_row("SELECT COUNT(*) FROM approval_requests", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
