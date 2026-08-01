//! Durable local-first observation intake and lifecycle receipts.

use std::path::Path;
use std::sync::Mutex;

use fabric::protocol::memory::{
    MemoryIntakeStatusV1, MemoryLifecycleReceiptV1, MemoryLifecycleStateV1,
    MemoryObservationKindV1, MemoryObservationReceiptV1, MemoryProjectionStateV1,
    MemoryProtocolValidationError, MemoryScorecardV1, MemorySensitivityV1, MemoryWorkspaceStateV1,
    MAX_MEMORY_ID_BYTES, MAX_MEMORY_SOURCE_REFS, MAX_MEMORY_SOURCE_REF_BYTES,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::WorkspaceMemoryKey;

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
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedMemoryObservation {
    pub observation_id: String,
    pub client_session_id: String,
    pub client_turn_id: Option<String>,
    pub kind: MemoryObservationKindV1,
    pub content: String,
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
            || self.content.len() > fabric::protocol::memory::MAX_MEMORY_CONTENT_BYTES
        {
            return Err(MemoryProtocolValidationError(
                "content is empty or exceeds byte limit".into(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl MemoryIntakeLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryIntakeError> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, MemoryIntakeError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, MemoryIntakeError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(INTAKE_SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
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
                serde_json::to_string(observation)?,
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
        if record_ids.len() > fabric::protocol::memory::MAX_MEMORY_RECALL_ITEMS
            || record_ids.iter().any(|id| {
                id.trim().is_empty() || id.len() > fabric::protocol::memory::MAX_MEMORY_ID_BYTES
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
        let receipt = MemoryLifecycleReceiptV1 {
            durable_intake_id: durable_intake_id.to_owned(),
            revision: current.revision + 1,
            state: update.state,
            resulting_record_ids,
            remote_receipt_ids,
            scorecard: update.scorecard.or(current.scorecard),
            reason_codes: update.reason_codes,
            terminal_at: update.terminal_at,
        };
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
}

fn observation_hash(observation: &GovernedMemoryObservation) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Material<'a> {
        observation_id: &'a str,
        client_session_id: &'a str,
        client_turn_id: &'a Option<String>,
        kind: MemoryObservationKindV1,
        content: &'a str,
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
