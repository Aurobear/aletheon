//! Durable local-first observation intake and lifecycle receipts.

use std::path::Path;
use std::sync::Mutex;

use ::contracts::protocol::memory::{
    MemoryIntakeStatusV1, MemoryLifecycleReceiptV1, MemoryLifecycleStateV1,
    MemoryObservationKindV1, MemoryObservationReceiptV1, MemoryProjectionStateV1,
    MemoryProtocolValidationError, MemoryScorecardV1, MemorySensitivityV1, MemoryWorkspaceStateV1,
    MAX_MEMORY_ID_BYTES, MAX_MEMORY_SOURCE_REFS, MAX_MEMORY_SOURCE_REF_BYTES,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::WorkspaceMemoryKey;

pub const DEFAULT_MAX_MEMORY_INTAKES: usize = 10_000;
pub const DEFAULT_MAX_MEMORY_INTAKE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryIntakeLimits {
    pub max_rows: usize,
    pub max_payload_bytes: usize,
}

impl Default for MemoryIntakeLimits {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_MAX_MEMORY_INTAKES,
            max_payload_bytes: DEFAULT_MAX_MEMORY_INTAKE_BYTES,
        }
    }
}

impl MemoryIntakeLimits {
    fn validate(self) -> Result<Self, MemoryProtocolValidationError> {
        if self.max_rows == 0
            || self.max_rows > 100_000
            || self.max_payload_bytes < ::contracts::protocol::memory::MAX_MEMORY_CONTENT_BYTES
            || self.max_payload_bytes > 2 * 1024 * 1024 * 1024
        {
            return Err(MemoryProtocolValidationError(
                "memory intake limits are invalid".into(),
            ));
        }
        Ok(self)
    }
}

const INTAKE_SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA synchronous=FULL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS memory_intakes(
  durable_intake_id TEXT PRIMARY KEY,
  principal_id TEXT NOT NULL,
  workspace_key TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  observation_json TEXT NOT NULL,
  observed_at_ms INTEGER NOT NULL,
  UNIQUE(principal_id, workspace_key, observation_id)
);
CREATE INDEX IF NOT EXISTS idx_memory_intakes_authority
  ON memory_intakes(principal_id, workspace_key, durable_intake_id);
CREATE TABLE IF NOT EXISTS memory_lifecycle_receipts(
  durable_intake_id TEXT NOT NULL REFERENCES memory_intakes(durable_intake_id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  state TEXT NOT NULL,
  receipt_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  PRIMARY KEY(durable_intake_id, revision)
);
CREATE INDEX IF NOT EXISTS idx_memory_lifecycle_latest
  ON memory_lifecycle_receipts(durable_intake_id, revision DESC);
CREATE TABLE IF NOT EXISTS visible_memory_records(
  principal_id TEXT NOT NULL,
  workspace_key TEXT NOT NULL,
  record_id TEXT NOT NULL,
  last_seen_at_ms INTEGER NOT NULL,
  PRIMARY KEY(principal_id, workspace_key, record_id)
);
CREATE TABLE IF NOT EXISTS memory_maintenance_leases(
  phase TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  watermark TEXT NOT NULL,
  durable_intake_id TEXT NOT NULL UNIQUE REFERENCES memory_intakes(durable_intake_id) ON DELETE CASCADE,
  lease_token TEXT NOT NULL UNIQUE,
  owner_id TEXT NOT NULL,
  claimed_revision INTEGER NOT NULL,
  lease_expires_at_ms INTEGER NOT NULL,
  PRIMARY KEY(phase, scope_key, watermark)
);
CREATE INDEX IF NOT EXISTS idx_memory_maintenance_lease_expiry
  ON memory_maintenance_leases(lease_expires_at_ms);
CREATE TABLE IF NOT EXISTS memory_maintenance_deferrals(
  durable_intake_id TEXT PRIMARY KEY REFERENCES memory_intakes(durable_intake_id) ON DELETE CASCADE,
  retry_not_before_ms INTEGER NOT NULL,
  reason_code TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_maintenance_settlements(
  settlement_id TEXT PRIMARY KEY,
  durable_intake_id TEXT NOT NULL REFERENCES memory_intakes(durable_intake_id) ON DELETE CASCADE,
  request_hash TEXT NOT NULL,
  receipt_json TEXT NOT NULL,
  settled_at_ms INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedMemoryObservation {
    pub observation_id: String,
    pub client_session_id: String,
    pub client_turn_id: Option<String>,
    pub kind: MemoryObservationKindV1,
    pub content: String,
    /// Installation-keyed fingerprint of the original client payload. The raw
    /// payload is never persisted, while conflicting idempotency reuse remains
    /// distinguishable after two values scrub to the same redaction marker.
    pub content_fingerprint: String,
    pub scrub_policy_version: u32,
    pub scrub_redactions: u32,
    pub occurred_at: Option<String>,
    pub source_refs: Vec<String>,
    pub sensitivity: MemorySensitivityV1,
    pub explicit_user_action: bool,
    pub principal_id: String,
    pub workspace_key: WorkspaceMemoryKey,
    pub connection_kind: String,
    pub observed_at_ms: i64,
}

impl GovernedMemoryObservation {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        for (name, value) in [
            ("observation_id", self.observation_id.as_str()),
            ("client_session_id", self.client_session_id.as_str()),
            ("principal_id", self.principal_id.as_str()),
            ("connection_kind", self.connection_kind.as_str()),
        ] {
            if value.trim().is_empty() || value.len() > MAX_MEMORY_ID_BYTES {
                return Err(MemoryProtocolValidationError(format!(
                    "{name} is empty or exceeds byte limit"
                )));
            }
        }
        if self
            .client_turn_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.len() > MAX_MEMORY_ID_BYTES)
        {
            return Err(MemoryProtocolValidationError(
                "client_turn_id is empty or exceeds byte limit".into(),
            ));
        }
        if self.content.trim().is_empty()
            || self.content.len() > ::contracts::protocol::memory::MAX_MEMORY_CONTENT_BYTES
        {
            return Err(MemoryProtocolValidationError(
                "content is empty or exceeds byte limit".into(),
            ));
        }
        if self.content_fingerprint.len() != 77
            || !self.content_fingerprint.starts_with("keyed-sha256:")
            || !self.content_fingerprint[13..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.scrub_policy_version == 0
            || self.scrub_redactions as usize
                > ::contracts::protocol::memory::MAX_MEMORY_CONTENT_BYTES
        {
            return Err(MemoryProtocolValidationError(
                "memory scrub evidence is invalid".into(),
            ));
        }
        if self.source_refs.len() > MAX_MEMORY_SOURCE_REFS
            || self
                .source_refs
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > MAX_MEMORY_SOURCE_REF_BYTES)
        {
            return Err(MemoryProtocolValidationError(
                "source_refs is invalid or exceeds its limit".into(),
            ));
        }
        if self.observed_at_ms < 0 {
            return Err(MemoryProtocolValidationError(
                "observed_at_ms cannot be negative".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLifecycleUpdate {
    pub expected_revision: u64,
    pub state: MemoryLifecycleStateV1,
    pub resulting_record_ids: Vec<String>,
    pub remote_receipt_ids: Vec<String>,
    pub scorecard: Option<MemoryScorecardV1>,
    pub reason_codes: Vec<String>,
    pub terminal_at: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryMaintenancePhase {
    IntakeEvaluation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMaintenanceLease {
    pub phase: MemoryMaintenancePhase,
    pub scope_key: String,
    pub watermark: String,
    pub durable_intake_id: String,
    pub lease_token: String,
    pub owner_id: String,
    pub claimed_revision: u64,
    pub lease_expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMaintenanceClaim {
    pub lease: MemoryMaintenanceLease,
    pub observation: GovernedMemoryObservation,
    pub lifecycle: MemoryLifecycleReceiptV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMaintenanceStatus {
    pub pending_items: u64,
    pub active_leases: u64,
    pub expired_leases: u64,
    pub oldest_pending_age_ms: Option<u64>,
}

impl MemoryLifecycleUpdate {
    pub fn new(expected_revision: u64, state: MemoryLifecycleStateV1) -> Self {
        Self {
            expected_revision,
            state,
            resulting_record_ids: Vec::new(),
            remote_receipt_ids: Vec::new(),
            scorecard: None,
            reason_codes: Vec::new(),
            terminal_at: None,
            created_at_ms: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryIntakeError {
    #[error(transparent)]
    Validation(#[from] MemoryProtocolValidationError),
    #[error("observation id was reused with different content or authority")]
    IdempotencyConflict,
    #[error("memory intake was not found for this authority")]
    NotFound,
    #[error("memory lifecycle revision conflict")]
    RevisionConflict,
    #[error("memory intake capacity exceeded")]
    Capacity,
    #[error("memory maintenance lease is unavailable or no longer authoritative")]
    LeaseUnavailable,
    #[error("memory maintenance settlement id was reused with different content")]
    SettlementConflict,
    #[error("invalid memory lifecycle transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: MemoryLifecycleStateV1,
        to: MemoryLifecycleStateV1,
    },
    #[error(transparent)]
    Storage(#[from] rusqlite::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

pub struct MemoryIntakeLedger {
    connection: Mutex<Connection>,
    limits: MemoryIntakeLimits,
}

impl MemoryIntakeLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryIntakeError> {
        Self::open_with_limits(path, MemoryIntakeLimits::default())
    }

    pub fn open_with_limits(
        path: impl AsRef<Path>,
        limits: MemoryIntakeLimits,
    ) -> Result<Self, MemoryIntakeError> {
        Self::from_connection(Connection::open(path)?, limits)
    }

    pub fn open_in_memory() -> Result<Self, MemoryIntakeError> {
        Self::from_connection(Connection::open_in_memory()?, MemoryIntakeLimits::default())
    }

    fn from_connection(
        connection: Connection,
        limits: MemoryIntakeLimits,
    ) -> Result<Self, MemoryIntakeError> {
        let limits = limits.validate()?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(INTAKE_SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
        })
    }

    pub fn observe(
        &self,
        observation: &GovernedMemoryObservation,
    ) -> Result<MemoryObservationReceiptV1, MemoryIntakeError> {
        observation.validate()?;
        let request_hash = observation_hash(observation)?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = transaction
            .query_row(
                "SELECT durable_intake_id,request_hash FROM memory_intakes
                 WHERE principal_id=?1 AND workspace_key=?2 AND observation_id=?3",
                params![
                    observation.principal_id,
                    observation.workspace_key.as_str(),
                    observation.observation_id
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((durable_intake_id, stored_hash)) = prior {
            if stored_hash != request_hash {
                return Err(MemoryIntakeError::IdempotencyConflict);
            }
            return Ok(observation_receipt(
                &observation.observation_id,
                durable_intake_id,
                MemoryIntakeStatusV1::Duplicate,
            ));
        }

        let observation_json = serde_json::to_string(observation)?;
        let (row_count, payload_bytes): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(SUM(length(observation_json)),0) FROM memory_intakes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if usize::try_from(row_count).unwrap_or(usize::MAX) >= self.limits.max_rows
            || usize::try_from(payload_bytes)
                .unwrap_or(usize::MAX)
                .saturating_add(observation_json.len())
                > self.limits.max_payload_bytes
        {
            return Err(MemoryIntakeError::Capacity);
        }

        let durable_intake_id = format!("intake:{}", uuid::Uuid::new_v4());
        transaction.execute(
            "INSERT INTO memory_intakes(
               durable_intake_id,principal_id,workspace_key,observation_id,
               request_hash,observation_json,observed_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                durable_intake_id,
                observation.principal_id,
                observation.workspace_key.as_str(),
                observation.observation_id,
                request_hash,
                observation_json,
                observation.observed_at_ms
            ],
        )?;
        let lifecycle = MemoryLifecycleReceiptV1 {
            durable_intake_id: durable_intake_id.clone(),
            revision: 1,
            state: MemoryLifecycleStateV1::Observed,
            resulting_record_ids: Vec::new(),
            remote_receipt_ids: Vec::new(),
            scorecard: None,
            reason_codes: Vec::new(),
            terminal_at: None,
        };
        transaction.execute(
            "INSERT INTO memory_lifecycle_receipts(
               durable_intake_id,revision,state,receipt_json,created_at_ms
             ) VALUES(?1,1,'observed',?2,?3)",
            params![
                durable_intake_id,
                serde_json::to_string(&lifecycle)?,
                observation.observed_at_ms
            ],
        )?;
        transaction.commit()?;
        Ok(observation_receipt(
            &observation.observation_id,
            durable_intake_id,
            MemoryIntakeStatusV1::Observed,
        ))
    }

    pub fn receipt(
        &self,
        principal_id: &str,
        workspace_key: &str,
        durable_intake_id: &str,
    ) -> Result<Option<MemoryLifecycleReceiptV1>, MemoryIntakeError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let json = connection
            .query_row(
                "SELECT lifecycle.receipt_json
                 FROM memory_intakes AS intake
                 JOIN memory_lifecycle_receipts AS lifecycle
                   ON lifecycle.durable_intake_id=intake.durable_intake_id
                 WHERE intake.principal_id=?1 AND intake.workspace_key=?2
                   AND intake.durable_intake_id=?3
                 ORDER BY lifecycle.revision DESC LIMIT 1",
                params![principal_id, workspace_key, durable_intake_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(MemoryIntakeError::from))
            .transpose()
    }

    /// Read an authoritative receipt by host principal when the wire contract
    /// carries only the durable intake ID. The opaque random ID remains scoped
    /// to its authenticated principal; workspace authority is never accepted
    /// from the client.
    pub fn receipt_for_principal(
        &self,
        principal_id: &str,
        durable_intake_id: &str,
    ) -> Result<Option<MemoryLifecycleReceiptV1>, MemoryIntakeError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let json = connection
            .query_row(
                "SELECT lifecycle.receipt_json
                 FROM memory_intakes AS intake
                 JOIN memory_lifecycle_receipts AS lifecycle
                   ON lifecycle.durable_intake_id=intake.durable_intake_id
                 WHERE intake.principal_id=?1 AND intake.durable_intake_id=?2
                 ORDER BY lifecycle.revision DESC LIMIT 1",
                params![principal_id, durable_intake_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(MemoryIntakeError::from))
            .transpose()
    }

    /// Persist the exact record IDs exposed by an authorized gateway recall so
    /// later feedback can prove the target was visible in the caller's host-
    /// derived principal/workspace ancestry. Client content never grants this
    /// visibility.
    pub fn remember_visible_records(
        &self,
        principal_id: &str,
        workspace_key: &str,
        record_ids: &[String],
        seen_at_ms: i64,
    ) -> Result<(), MemoryIntakeError> {
        validate_authority_key("principal_id", principal_id)?;
        WorkspaceMemoryKey::from_verified(workspace_key.to_owned())
            .map_err(|error| MemoryProtocolValidationError(error.to_string()))?;
        if record_ids.len() > ::contracts::protocol::memory::MAX_MEMORY_RECALL_ITEMS
            || record_ids.iter().any(|id| {
                id.trim().is_empty()
                    || id.len() > ::contracts::protocol::memory::MAX_MEMORY_ID_BYTES
            })
            || seen_at_ms < 0
        {
            return Err(MemoryProtocolValidationError(
                "visible record grant is invalid or exceeds its limit".into(),
            )
            .into());
        }
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for record_id in record_ids {
            transaction.execute(
                "INSERT INTO visible_memory_records(
                   principal_id,workspace_key,record_id,last_seen_at_ms
                 ) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(principal_id,workspace_key,record_id)
                 DO UPDATE SET last_seen_at_ms=excluded.last_seen_at_ms",
                params![principal_id, workspace_key, record_id, seen_at_ms],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn is_record_visible(
        &self,
        principal_id: &str,
        workspace_key: &str,
        record_id: &str,
    ) -> Result<bool, MemoryIntakeError> {
        validate_authority_key("principal_id", principal_id)?;
        WorkspaceMemoryKey::from_verified(workspace_key.to_owned())
            .map_err(|error| MemoryProtocolValidationError(error.to_string()))?;
        validate_authority_key("record_id", record_id)?;
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let exists = connection
            .query_row(
                "SELECT 1 FROM visible_memory_records
                 WHERE principal_id=?1 AND workspace_key=?2 AND record_id=?3",
                params![principal_id, workspace_key, record_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }

    /// Atomically claim the oldest eligible intake. The transition from
    /// `observed` to `evaluating` and the lease insert share one transaction,
    /// so no worker can observe an evaluating row without recoverable lease
    /// authority.
    pub fn claim_next_maintenance(
        &self,
        owner_id: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<MemoryMaintenanceClaim>, MemoryIntakeError> {
        validate_authority_key("owner_id", owner_id)?;
        if now_ms < 0 || !(1_000..=300_000).contains(&lease_ms) {
            return Err(MemoryProtocolValidationError(
                "memory maintenance lease timing is invalid".into(),
            )
            .into());
        }
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidate: Option<(String, String)> = transaction
            .query_row(
                "WITH latest AS (
                   SELECT durable_intake_id,MAX(revision) AS revision
                   FROM memory_lifecycle_receipts GROUP BY durable_intake_id
                 )
                 SELECT intake.observation_json,lifecycle.receipt_json
                 FROM memory_intakes AS intake
                 JOIN latest ON latest.durable_intake_id=intake.durable_intake_id
                 JOIN memory_lifecycle_receipts AS lifecycle
                   ON lifecycle.durable_intake_id=latest.durable_intake_id
                  AND lifecycle.revision=latest.revision
                 LEFT JOIN memory_maintenance_leases AS lease
                   ON lease.durable_intake_id=intake.durable_intake_id
                 LEFT JOIN memory_maintenance_deferrals AS deferred
                   ON deferred.durable_intake_id=intake.durable_intake_id
                 WHERE lifecycle.state IN ('observed','evaluating')
                   AND (lease.durable_intake_id IS NULL OR lease.lease_expires_at_ms<=?1)
                   AND (deferred.durable_intake_id IS NULL OR deferred.retry_not_before_ms<=?1)
                 ORDER BY
                   CASE WHEN json_extract(intake.observation_json,'$.kind')='feedback'
                        THEN 0 ELSE 1 END,
                   intake.observed_at_ms,intake.durable_intake_id
                 LIMIT 1",
                params![now_ms],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((observation_json, lifecycle_json)) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        let observation: GovernedMemoryObservation = serde_json::from_str(&observation_json)?;
        observation.validate()?;
        let current: MemoryLifecycleReceiptV1 = serde_json::from_str(&lifecycle_json)?;
        let lifecycle = if current.state == MemoryLifecycleStateV1::Observed {
            let lifecycle = MemoryLifecycleReceiptV1 {
                durable_intake_id: current.durable_intake_id.clone(),
                revision: current.revision + 1,
                state: MemoryLifecycleStateV1::Evaluating,
                resulting_record_ids: current.resulting_record_ids,
                remote_receipt_ids: current.remote_receipt_ids,
                scorecard: current.scorecard,
                reason_codes: Vec::new(),
                terminal_at: None,
            };
            transaction.execute(
                "INSERT INTO memory_lifecycle_receipts(
                   durable_intake_id,revision,state,receipt_json,created_at_ms
                 ) VALUES(?1,?2,'evaluating',?3,?4)",
                params![
                    lifecycle.durable_intake_id,
                    lifecycle.revision,
                    serde_json::to_string(&lifecycle)?,
                    now_ms
                ],
            )?;
            lifecycle
        } else {
            current
        };
        transaction.execute(
            "DELETE FROM memory_maintenance_leases
             WHERE durable_intake_id=?1 AND lease_expires_at_ms<=?2",
            params![lifecycle.durable_intake_id, now_ms],
        )?;
        transaction.execute(
            "DELETE FROM memory_maintenance_deferrals WHERE durable_intake_id=?1",
            params![lifecycle.durable_intake_id],
        )?;
        let lease = MemoryMaintenanceLease {
            phase: MemoryMaintenancePhase::IntakeEvaluation,
            scope_key: observation.workspace_key.as_str().to_owned(),
            watermark: lifecycle.durable_intake_id.clone(),
            durable_intake_id: lifecycle.durable_intake_id.clone(),
            lease_token: format!("memory-lease:{}", uuid::Uuid::new_v4()),
            owner_id: owner_id.to_owned(),
            claimed_revision: lifecycle.revision,
            lease_expires_at_ms: now_ms.saturating_add(lease_ms),
        };
        transaction.execute(
            "INSERT INTO memory_maintenance_leases(
               phase,scope_key,watermark,durable_intake_id,lease_token,
               owner_id,claimed_revision,lease_expires_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                maintenance_phase_name(lease.phase),
                lease.scope_key,
                lease.watermark,
                lease.durable_intake_id,
                lease.lease_token,
                lease.owner_id,
                lease.claimed_revision,
                lease.lease_expires_at_ms
            ],
        )?;
        transaction.commit()?;
        Ok(Some(MemoryMaintenanceClaim {
            lease,
            observation,
            lifecycle,
        }))
    }

    pub fn defer_maintenance(
        &self,
        lease: &MemoryMaintenanceLease,
        retry_not_before_ms: i64,
        reason_code: &str,
        now_ms: i64,
    ) -> Result<MemoryLifecycleReceiptV1, MemoryIntakeError> {
        validate_lease(lease)?;
        validate_authority_key("reason_code", reason_code)?;
        if now_ms < 0 || retry_not_before_ms <= now_ms {
            return Err(MemoryProtocolValidationError(
                "memory maintenance deferral timing is invalid".into(),
            )
            .into());
        }
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = authoritative_lease_receipt(&transaction, lease, now_ms)?;
        transaction.execute(
            "INSERT INTO memory_maintenance_deferrals(
               durable_intake_id,retry_not_before_ms,reason_code
             ) VALUES(?1,?2,?3)
             ON CONFLICT(durable_intake_id) DO UPDATE SET
               retry_not_before_ms=excluded.retry_not_before_ms,
               reason_code=excluded.reason_code",
            params![lease.durable_intake_id, retry_not_before_ms, reason_code],
        )?;
        transaction.execute(
            "DELETE FROM memory_maintenance_leases WHERE lease_token=?1",
            params![lease.lease_token],
        )?;
        transaction.commit()?;
        Ok(current)
    }

    pub fn settle_maintenance(
        &self,
        lease: &MemoryMaintenanceLease,
        settlement_id: &str,
        update: MemoryLifecycleUpdate,
        now_ms: i64,
    ) -> Result<MemoryLifecycleReceiptV1, MemoryIntakeError> {
        validate_lease(lease)?;
        validate_authority_key("settlement_id", settlement_id)?;
        validate_update(&update)?;
        if now_ms < 0
            || !matches!(
                update.state,
                MemoryLifecycleStateV1::Rejected | MemoryLifecycleStateV1::PromotedLocal
            )
        {
            return Err(MemoryProtocolValidationError(
                "memory maintenance settlement is not terminal local evaluation".into(),
            )
            .into());
        }
        let request_hash = settlement_hash(lease, &update)?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = transaction
            .query_row(
                "SELECT request_hash,receipt_json FROM memory_maintenance_settlements
                 WHERE settlement_id=?1",
                params![settlement_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((prior_hash, receipt_json)) = prior {
            if prior_hash != request_hash {
                return Err(MemoryIntakeError::SettlementConflict);
            }
            return serde_json::from_str(&receipt_json).map_err(MemoryIntakeError::from);
        }
        let current = authoritative_lease_receipt(&transaction, lease, now_ms)?;
        if current.revision != update.expected_revision {
            return Err(MemoryIntakeError::RevisionConflict);
        }
        let receipt = next_lifecycle_receipt(&lease.durable_intake_id, current, update.clone())?;
        transaction.execute(
            "INSERT INTO memory_lifecycle_receipts(
               durable_intake_id,revision,state,receipt_json,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                lease.durable_intake_id,
                receipt.revision,
                lifecycle_state_name(receipt.state),
                serde_json::to_string(&receipt)?,
                update.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO memory_maintenance_settlements(
               settlement_id,durable_intake_id,request_hash,receipt_json,settled_at_ms
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                settlement_id,
                lease.durable_intake_id,
                request_hash,
                serde_json::to_string(&receipt)?,
                now_ms
            ],
        )?;
        transaction.execute(
            "DELETE FROM memory_maintenance_leases WHERE lease_token=?1",
            params![lease.lease_token],
        )?;
        transaction.execute(
            "DELETE FROM memory_maintenance_deferrals WHERE durable_intake_id=?1",
            params![lease.durable_intake_id],
        )?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub fn maintenance_status(
        &self,
        now_ms: i64,
    ) -> Result<MemoryMaintenanceStatus, MemoryIntakeError> {
        if now_ms < 0 {
            return Err(MemoryProtocolValidationError(
                "memory maintenance status time is invalid".into(),
            )
            .into());
        }
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (pending, oldest): (i64, Option<i64>) = connection.query_row(
            "WITH latest AS (
               SELECT durable_intake_id,MAX(revision) AS revision
               FROM memory_lifecycle_receipts GROUP BY durable_intake_id
             )
             SELECT COUNT(*),MIN(intake.observed_at_ms)
             FROM memory_intakes AS intake
             JOIN latest ON latest.durable_intake_id=intake.durable_intake_id
             JOIN memory_lifecycle_receipts AS lifecycle
               ON lifecycle.durable_intake_id=latest.durable_intake_id
              AND lifecycle.revision=latest.revision
             WHERE lifecycle.state IN ('observed','evaluating')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (active, expired): (i64, i64) = connection.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN lease_expires_at_ms>?1 THEN 1 ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN lease_expires_at_ms<=?1 THEN 1 ELSE 0 END),0)
             FROM memory_maintenance_leases",
            params![now_ms],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(MemoryMaintenanceStatus {
            pending_items: pending.try_into().unwrap_or(u64::MAX),
            active_leases: active.try_into().unwrap_or(u64::MAX),
            expired_leases: expired.try_into().unwrap_or(u64::MAX),
            oldest_pending_age_ms: oldest.map(|value| now_ms.saturating_sub(value) as u64),
        })
    }

    pub fn transition(
        &self,
        principal_id: &str,
        workspace_key: &str,
        durable_intake_id: &str,
        update: MemoryLifecycleUpdate,
    ) -> Result<MemoryLifecycleReceiptV1, MemoryIntakeError> {
        validate_update(&update)?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_json: Option<String> = transaction
            .query_row(
                "SELECT lifecycle.receipt_json
                 FROM memory_intakes AS intake
                 JOIN memory_lifecycle_receipts AS lifecycle
                   ON lifecycle.durable_intake_id=intake.durable_intake_id
                 WHERE intake.principal_id=?1 AND intake.workspace_key=?2
                   AND intake.durable_intake_id=?3
                 ORDER BY lifecycle.revision DESC LIMIT 1",
                params![principal_id, workspace_key, durable_intake_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current_json) = current_json else {
            return Err(MemoryIntakeError::NotFound);
        };
        let current: MemoryLifecycleReceiptV1 = serde_json::from_str(&current_json)?;
        let receipt = next_lifecycle_receipt(durable_intake_id, current, update.clone())?;
        transaction.execute(
            "INSERT INTO memory_lifecycle_receipts(
               durable_intake_id,revision,state,receipt_json,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                durable_intake_id,
                receipt.revision,
                lifecycle_state_name(receipt.state),
                serde_json::to_string(&receipt)?,
                update.created_at_ms
            ],
        )?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub fn pending_projection_receipts(
        &self,
        limit: usize,
    ) -> Result<Vec<(GovernedMemoryObservation, MemoryLifecycleReceiptV1)>, MemoryIntakeError> {
        if limit == 0 || limit > 1_000 {
            return Err(
                MemoryProtocolValidationError("projection query limit is invalid".into()).into(),
            );
        }
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut statement = connection.prepare(
            "WITH latest AS (
               SELECT durable_intake_id,MAX(revision) AS revision
               FROM memory_lifecycle_receipts GROUP BY durable_intake_id
             )
             SELECT intake.observation_json,lifecycle.receipt_json
             FROM memory_intakes AS intake
             JOIN latest ON latest.durable_intake_id=intake.durable_intake_id
             JOIN memory_lifecycle_receipts AS lifecycle
               ON lifecycle.durable_intake_id=latest.durable_intake_id
              AND lifecycle.revision=latest.revision
             WHERE lifecycle.state='projection_queued'
             ORDER BY intake.observed_at_ms,intake.durable_intake_id LIMIT ?1",
        )?;
        let values = statement
            .query_map([limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|(observation, receipt)| {
                Ok((
                    serde_json::from_str(&observation)?,
                    serde_json::from_str(&receipt)?,
                ))
            })
            .collect()
    }
}

fn validate_lease(lease: &MemoryMaintenanceLease) -> Result<(), MemoryProtocolValidationError> {
    WorkspaceMemoryKey::from_verified(lease.scope_key.clone())
        .map_err(|error| MemoryProtocolValidationError(error.to_string()))?;
    validate_authority_key("watermark", &lease.watermark)?;
    validate_authority_key("durable_intake_id", &lease.durable_intake_id)?;
    validate_authority_key("lease_token", &lease.lease_token)?;
    validate_authority_key("owner_id", &lease.owner_id)?;
    if lease.claimed_revision == 0 || lease.lease_expires_at_ms < 0 {
        return Err(MemoryProtocolValidationError(
            "memory maintenance lease is invalid".into(),
        ));
    }
    Ok(())
}

fn authoritative_lease_receipt(
    transaction: &Transaction<'_>,
    lease: &MemoryMaintenanceLease,
    now_ms: i64,
) -> Result<MemoryLifecycleReceiptV1, MemoryIntakeError> {
    let receipt_json = transaction
        .query_row(
            "SELECT lifecycle.receipt_json
             FROM memory_maintenance_leases AS lease
             JOIN memory_lifecycle_receipts AS lifecycle
               ON lifecycle.durable_intake_id=lease.durable_intake_id
              AND lifecycle.revision=lease.claimed_revision
             WHERE lease.phase=?1 AND lease.scope_key=?2 AND lease.watermark=?3
               AND lease.durable_intake_id=?4 AND lease.lease_token=?5
               AND lease.owner_id=?6 AND lease.claimed_revision=?7
               AND lease.lease_expires_at_ms>?8",
            params![
                maintenance_phase_name(lease.phase),
                lease.scope_key,
                lease.watermark,
                lease.durable_intake_id,
                lease.lease_token,
                lease.owner_id,
                lease.claimed_revision,
                now_ms
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(MemoryIntakeError::LeaseUnavailable)?;
    serde_json::from_str(&receipt_json).map_err(MemoryIntakeError::from)
}

fn next_lifecycle_receipt(
    durable_intake_id: &str,
    current: MemoryLifecycleReceiptV1,
    update: MemoryLifecycleUpdate,
) -> Result<MemoryLifecycleReceiptV1, MemoryIntakeError> {
    if current.revision != update.expected_revision {
        return Err(MemoryIntakeError::RevisionConflict);
    }
    if !allows_transition(current.state, update.state) {
        return Err(MemoryIntakeError::InvalidTransition {
            from: current.state,
            to: update.state,
        });
    }
    let mut resulting_record_ids = current.resulting_record_ids;
    resulting_record_ids.extend(update.resulting_record_ids);
    normalize_ids(&mut resulting_record_ids);
    let mut remote_receipt_ids = current.remote_receipt_ids;
    remote_receipt_ids.extend(update.remote_receipt_ids);
    normalize_ids(&mut remote_receipt_ids);
    Ok(MemoryLifecycleReceiptV1 {
        durable_intake_id: durable_intake_id.to_owned(),
        revision: current.revision + 1,
        state: update.state,
        resulting_record_ids,
        remote_receipt_ids,
        scorecard: update.scorecard.or(current.scorecard),
        reason_codes: update.reason_codes,
        terminal_at: update.terminal_at,
    })
}

fn settlement_hash(
    lease: &MemoryMaintenanceLease,
    update: &MemoryLifecycleUpdate,
) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&(&lease.durable_intake_id, update))?)
    ))
}

fn maintenance_phase_name(phase: MemoryMaintenancePhase) -> &'static str {
    match phase {
        MemoryMaintenancePhase::IntakeEvaluation => "intake_evaluation",
    }
}

fn observation_hash(observation: &GovernedMemoryObservation) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Material<'a> {
        observation_id: &'a str,
        client_session_id: &'a str,
        client_turn_id: &'a Option<String>,
        kind: MemoryObservationKindV1,
        content: &'a str,
        content_fingerprint: &'a str,
        scrub_policy_version: u32,
        scrub_redactions: u32,
        occurred_at: &'a Option<String>,
        source_refs: &'a [String],
        sensitivity: MemorySensitivityV1,
        explicit_user_action: bool,
        principal_id: &'a str,
        workspace_key: &'a str,
        connection_kind: &'a str,
    }
    let material = Material {
        observation_id: &observation.observation_id,
        client_session_id: &observation.client_session_id,
        client_turn_id: &observation.client_turn_id,
        kind: observation.kind,
        content: &observation.content,
        content_fingerprint: &observation.content_fingerprint,
        scrub_policy_version: observation.scrub_policy_version,
        scrub_redactions: observation.scrub_redactions,
        occurred_at: &observation.occurred_at,
        source_refs: &observation.source_refs,
        sensitivity: observation.sensitivity,
        explicit_user_action: observation.explicit_user_action,
        principal_id: &observation.principal_id,
        workspace_key: observation.workspace_key.as_str(),
        connection_kind: &observation.connection_kind,
    };
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&material)?)
    ))
}

fn observation_receipt(
    observation_id: &str,
    durable_intake_id: String,
    intake_status: MemoryIntakeStatusV1,
) -> MemoryObservationReceiptV1 {
    MemoryObservationReceiptV1 {
        observation_id: observation_id.to_owned(),
        durable_intake_id,
        intake_status,
        workspace_state: MemoryWorkspaceStateV1::LocalOnly,
        projection_state: MemoryProjectionStateV1::PendingEvaluation,
        reason_code: None,
    }
}

fn validate_update(update: &MemoryLifecycleUpdate) -> Result<(), MemoryProtocolValidationError> {
    if update.expected_revision == 0 {
        return Err(MemoryProtocolValidationError(
            "expected_revision must be positive".into(),
        ));
    }
    if update.created_at_ms < 0 {
        return Err(MemoryProtocolValidationError(
            "created_at_ms cannot be negative".into(),
        ));
    }
    for values in [
        update.resulting_record_ids.as_slice(),
        update.remote_receipt_ids.as_slice(),
        update.reason_codes.as_slice(),
    ] {
        if values.len() > 256
            || values
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > MAX_MEMORY_ID_BYTES)
        {
            return Err(MemoryProtocolValidationError(
                "lifecycle identifier list is invalid or exceeds its limit".into(),
            ));
        }
    }
    Ok(())
}

fn validate_authority_key(name: &str, value: &str) -> Result<(), MemoryProtocolValidationError> {
    if value.trim().is_empty() || value.len() > MAX_MEMORY_ID_BYTES {
        return Err(MemoryProtocolValidationError(format!(
            "{name} is empty or exceeds byte limit"
        )));
    }
    Ok(())
}

fn normalize_ids(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn allows_transition(from: MemoryLifecycleStateV1, to: MemoryLifecycleStateV1) -> bool {
    use MemoryLifecycleStateV1::*;
    matches!(
        (from, to),
        (Observed, Evaluating | Rejected | Tombstoned)
            | (Evaluating, Rejected | PromotedLocal | Tombstoned)
            | (Rejected, Tombstoned)
            | (PromotedLocal, ProjectionQueued | Superseded | Tombstoned)
            | (
                ProjectionQueued,
                ProjectedRemote | ProjectionFailed | Tombstoned
            )
            | (ProjectedRemote, Superseded | Tombstoned)
            | (ProjectionFailed, ProjectionQueued | Superseded | Tombstoned)
            | (Superseded, Tombstoned)
    )
}

fn lifecycle_state_name(state: MemoryLifecycleStateV1) -> &'static str {
    use MemoryLifecycleStateV1::*;
    match state {
        Observed => "observed",
        Evaluating => "evaluating",
        Rejected => "rejected",
        PromotedLocal => "promoted_local",
        ProjectionQueued => "projection_queued",
        ProjectedRemote => "projected_remote",
        ProjectionFailed => "projection_failed",
        Superseded => "superseded",
        Tombstoned => "tombstoned",
    }
}
