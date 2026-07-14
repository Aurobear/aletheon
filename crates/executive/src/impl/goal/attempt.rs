//! Durable Goal attempt lifecycle and restart recovery.

use super::ObjectiveStore;
use anyhow::{bail, Context, Result};
use fabric::{
    AttemptEvidence, AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, FailureClass, GoalId,
    RuntimeFailure, RuntimeId, RuntimeResult,
};
use rusqlite::{params, OptionalExtension, Row, Transaction};

const ATTEMPT_COLS: &str = "attempt_id, objective_id, sequence, runtime_id, role, status, \
input_json, output_json, failure_json, evidence_json, usage_json, started_at, ended_at";
const MAX_ATTEMPT_FIELD_BYTES: usize = 64 * 1024;

/// Fully materialized durable runtime attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalAttempt {
    pub id: AttemptId,
    pub goal_id: GoalId,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub role: CognitiveRole,
    pub status: AttemptStatus,
    pub input: serde_json::Value,
    pub output: Option<RuntimeResult>,
    pub failure: Option<RuntimeFailure>,
    pub evidence: Vec<AttemptEvidence>,
    pub usage: AttemptUsage,
    pub started_at: String,
    pub ended_at: Option<String>,
}

impl ObjectiveStore {
    /// Persist a running attempt and its Goal event atomically.
    pub fn begin_attempt(
        &self,
        goal_id: GoalId,
        sequence: u32,
        runtime_id: &RuntimeId,
        role: CognitiveRole,
        input: &serde_json::Value,
    ) -> Result<GoalAttempt> {
        self.begin_attempt_with_id(AttemptId::new(), goal_id, sequence, runtime_id, role, input)
    }

    /// Persist a running attempt with a coordinator-allocated immutable ID.
    /// Coding jobs need the ID in their runtime request before invocation.
    pub(crate) fn begin_attempt_with_id(
        &self,
        attempt_id: AttemptId,
        goal_id: GoalId,
        sequence: u32,
        runtime_id: &RuntimeId,
        role: CognitiveRole,
        input: &serde_json::Value,
    ) -> Result<GoalAttempt> {
        let input_json = serde_json::to_string(input)?;
        let role = wire_value(role)?;
        let tx = self.db.unchecked_transaction()?;
        ensure_goal_exists(&tx, goal_id)?;
        tx.execute(
            "INSERT INTO goal_attempts (
                attempt_id, objective_id, sequence, runtime_id, role, status,
                input_json, evidence_json, usage_json, started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, '[]', ?7, datetime('now'))",
            params![
                attempt_id.0.to_string(),
                goal_id.0,
                sequence,
                runtime_id.0,
                role,
                input_json,
                serde_json::to_string(&AttemptUsage::default())?,
            ],
        )
        .context("inserting running goal attempt")?;
        append_attempt_event(
            &tx,
            goal_id,
            "attempt_started",
            &serde_json::json!({
                "attempt_id": attempt_id.0,
                "sequence": sequence,
                "runtime_id": runtime_id.0,
                "role": role,
            }),
        )?;
        tx.commit()?;
        self.attempt(attempt_id)?
            .context("attempt disappeared after begin")
    }

    /// Finish a running attempt as success or failure in one transaction.
    pub fn finish_attempt(
        &self,
        attempt_id: AttemptId,
        outcome: Result<RuntimeResult, RuntimeFailure>,
    ) -> Result<GoalAttempt> {
        let tx = self.db.unchecked_transaction()?;
        let (goal_id, status) = attempt_identity(&tx, attempt_id)?;
        ensure_running(status)?;

        let (status, output_json, failure_json, evidence, usage, event_type) = match outcome {
            Ok(result) => {
                let result = result.bounded_for_persistence(MAX_ATTEMPT_FIELD_BYTES);
                (
                    AttemptStatus::Succeeded,
                    Some(serde_json::to_string(&result)?),
                    None,
                    result.evidence,
                    result.usage,
                    "attempt_succeeded",
                )
            }
            Err(failure) => {
                let failure = failure.bounded_for_persistence(MAX_ATTEMPT_FIELD_BYTES);
                (
                    AttemptStatus::Failed,
                    None,
                    Some(serde_json::to_string(&failure)?),
                    failure.evidence,
                    failure.usage,
                    "attempt_failed",
                )
            }
        };
        update_terminal_attempt(
            &tx,
            attempt_id,
            status,
            output_json.as_deref(),
            failure_json.as_deref(),
            &evidence,
            &usage,
        )?;
        append_attempt_event(
            &tx,
            goal_id,
            event_type,
            &serde_json::json!({"attempt_id": attempt_id.0}),
        )?;
        tx.commit()?;
        self.attempt(attempt_id)?
            .context("attempt disappeared after finish")
    }

    /// Explicitly cancel a running attempt. The supplied failure must be the
    /// cancellation class so usage/evidence from the interrupted runtime survive.
    pub fn cancel_attempt(
        &self,
        attempt_id: AttemptId,
        failure: RuntimeFailure,
    ) -> Result<GoalAttempt> {
        if failure.class != FailureClass::Cancelled {
            bail!("cancel_attempt requires FailureClass::Cancelled");
        }
        let failure = failure.bounded_for_persistence(MAX_ATTEMPT_FIELD_BYTES);
        let tx = self.db.unchecked_transaction()?;
        let (goal_id, status) = attempt_identity(&tx, attempt_id)?;
        ensure_running(status)?;
        let failure_json = serde_json::to_string(&failure)?;
        update_terminal_attempt(
            &tx,
            attempt_id,
            AttemptStatus::Cancelled,
            None,
            Some(&failure_json),
            &failure.evidence,
            &failure.usage,
        )?;
        append_attempt_event(
            &tx,
            goal_id,
            "attempt_cancelled",
            &serde_json::json!({"attempt_id": attempt_id.0}),
        )?;
        tx.commit()?;
        self.attempt(attempt_id)?
            .context("attempt disappeared after cancel")
    }

    /// List newest attempts for one Goal, ordered deterministically by sequence.
    pub fn attempts_for_goal(&self, goal_id: GoalId, limit: usize) -> Result<Vec<GoalAttempt>> {
        let sql = format!(
            "SELECT {ATTEMPT_COLS} FROM goal_attempts
             WHERE objective_id = ?1 ORDER BY sequence DESC LIMIT ?2"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let attempts = stmt
            .query_map(params![goal_id.0, limit as i64], map_attempt_row)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(anyhow::Error::from)?;
        Ok(attempts)
    }

    /// Mark every stale running attempt as explicitly cancelled after restart.
    /// No runtime invocation is performed here.
    pub fn recover_stale_attempts(&self) -> Result<Vec<GoalAttempt>> {
        let mut stmt = self
            .db
            .prepare("SELECT attempt_id FROM goal_attempts WHERE status = 'running' ORDER BY objective_id, sequence")?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut recovered = Vec::with_capacity(ids.len());
        for id in ids {
            let attempt_id = AttemptId(uuid::Uuid::parse_str(&id)?);
            recovered.push(self.cancel_attempt(
                attempt_id,
                RuntimeFailure {
                    class: FailureClass::Cancelled,
                    message: "stale running attempt cancelled during daemon recovery".into(),
                    retryable: false,
                    usage: AttemptUsage::default(),
                    evidence: vec![AttemptEvidence {
                        kind: "restart_recovery".into(),
                        summary: "attempt interrupted by daemon restart".into(),
                        content: "runtime was not re-invoked".into(),
                    }],
                },
            )?);
        }
        Ok(recovered)
    }

    pub(crate) fn attempt(&self, attempt_id: AttemptId) -> Result<Option<GoalAttempt>> {
        let sql = format!("SELECT {ATTEMPT_COLS} FROM goal_attempts WHERE attempt_id = ?1");
        self.db
            .query_row(&sql, params![attempt_id.0.to_string()], map_attempt_row)
            .optional()
            .map_err(Into::into)
    }

    /// Add bounded verifier evidence to an already terminal attempt so the
    /// next GoalFrame can carry deterministic failure context.
    pub(crate) fn append_attempt_evidence(
        &self,
        attempt_id: AttemptId,
        additional: &[AttemptEvidence],
    ) -> Result<GoalAttempt> {
        let mut attempt = self
            .attempt(attempt_id)?
            .context("attempt missing while appending verification evidence")?;
        if attempt.status == AttemptStatus::Running {
            bail!("cannot append verifier evidence to a running attempt");
        }
        attempt.evidence.extend(
            additional
                .iter()
                .map(|item| item.bounded_for_persistence(MAX_ATTEMPT_FIELD_BYTES)),
        );
        let evidence_json = serde_json::to_string(&attempt.evidence)?;
        if evidence_json.len() > MAX_ATTEMPT_FIELD_BYTES {
            bail!("combined attempt evidence exceeds persistence limit");
        }
        self.db.execute(
            "UPDATE goal_attempts SET evidence_json = ?1 WHERE attempt_id = ?2",
            params![evidence_json, attempt_id.0.to_string()],
        )?;
        self.attempt(attempt_id)?
            .context("attempt disappeared after appending verification evidence")
    }
}

fn ensure_goal_exists(tx: &Transaction<'_>, goal_id: GoalId) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM objectives WHERE objective_id = ?1)",
        params![goal_id.0],
        |row| row.get(0),
    )?;
    anyhow::ensure!(exists, "goal {} not found", goal_id.0);
    Ok(())
}

fn attempt_identity(
    tx: &Transaction<'_>,
    attempt_id: AttemptId,
) -> Result<(GoalId, AttemptStatus)> {
    let value = tx
        .query_row(
            "SELECT objective_id, status FROM goal_attempts WHERE attempt_id = ?1",
            params![attempt_id.0.to_string()],
            |row| Ok((GoalId(row.get(0)?), row.get::<_, String>(1)?)),
        )
        .optional()?
        .with_context(|| format!("attempt {} not found", attempt_id.0))?;
    Ok((value.0, parse_status(&value.1)?))
}

fn ensure_running(status: AttemptStatus) -> Result<()> {
    anyhow::ensure!(
        status == AttemptStatus::Running,
        "attempt is already terminal: {status:?}"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_terminal_attempt(
    tx: &Transaction<'_>,
    attempt_id: AttemptId,
    status: AttemptStatus,
    output_json: Option<&str>,
    failure_json: Option<&str>,
    evidence: &[AttemptEvidence],
    usage: &AttemptUsage,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE goal_attempts SET status = ?1, output_json = ?2, failure_json = ?3,
         evidence_json = ?4, usage_json = ?5, ended_at = datetime('now')
         WHERE attempt_id = ?6 AND status = 'running'",
        params![
            wire_value(status)?,
            output_json,
            failure_json,
            serde_json::to_string(evidence)?,
            serde_json::to_string(usage)?,
            attempt_id.0.to_string(),
        ],
    )?;
    anyhow::ensure!(changed == 1, "attempt terminal update lost a race");
    Ok(())
}

fn append_attempt_event(
    tx: &Transaction<'_>,
    goal_id: GoalId,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let version: u64 = tx.query_row(
        "SELECT version FROM objectives WHERE objective_id = ?1",
        params![goal_id.0],
        |row| row.get(0),
    )?;
    let next = version.saturating_add(1);
    let changed = tx.execute(
        "UPDATE objectives SET version = ?1, updated_at = datetime('now')
         WHERE objective_id = ?2 AND version = ?3",
        params![next, goal_id.0, version],
    )?;
    anyhow::ensure!(
        changed == 1,
        "goal version changed while recording attempt event"
    );
    tx.execute(
        "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![goal_id.0, next, event_type, serde_json::to_string(payload)?],
    )?;
    Ok(())
}

fn map_attempt_row(row: &Row<'_>) -> rusqlite::Result<GoalAttempt> {
    let attempt_id: String = row.get(0)?;
    let role: String = row.get(4)?;
    let status: String = row.get(5)?;
    let input_json: String = row.get(6)?;
    let output_json: Option<String> = row.get(7)?;
    let failure_json: Option<String> = row.get(8)?;
    let evidence_json: String = row.get(9)?;
    let usage_json: String = row.get(10)?;
    Ok(GoalAttempt {
        id: AttemptId(parse_uuid(&attempt_id)?),
        goal_id: GoalId(row.get(1)?),
        sequence: row.get(2)?,
        runtime_id: RuntimeId(row.get(3)?),
        role: parse_wire(&role)?,
        status: parse_status_sql(&status)?,
        input: parse_json(&input_json)?,
        output: output_json.as_deref().map(parse_json).transpose()?,
        failure: failure_json.as_deref().map(parse_json).transpose()?,
        evidence: parse_json(&evidence_json)?,
        usage: parse_json(&usage_json)?,
        started_at: row.get(11)?,
        ended_at: row.get(12)?,
    })
}

fn wire_value<T: serde::Serialize>(value: T) -> Result<String> {
    let json = serde_json::to_value(value)?;
    json.as_str()
        .map(str::to_owned)
        .context("wire enum did not serialize as a string")
}

fn parse_wire<T: serde::de::DeserializeOwned>(value: &str) -> rusqlite::Result<T> {
    parse_json(&serde_json::to_string(value).map_err(sql_conversion_error)?)
}

fn parse_status(value: &str) -> Result<AttemptStatus> {
    serde_json::from_value(serde_json::Value::String(value.into())).map_err(Into::into)
}

fn parse_status_sql(value: &str) -> rusqlite::Result<AttemptStatus> {
    parse_wire(value)
}

fn parse_uuid(value: &str) -> rusqlite::Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).map_err(sql_conversion_error)
}

fn parse_json<T: serde::de::DeserializeOwned>(value: &str) -> rusqlite::Result<T> {
    serde_json::from_str(value).map_err(sql_conversion_error)
}

fn sql_conversion_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{GoalSpec, PrincipalId};
    use tempfile::NamedTempFile;

    fn setup() -> (ObjectiveStore, NamedTempFile, GoalId) {
        let tmp = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(tmp.path()).unwrap();
        let goal = store
            .create_goal(
                &PrincipalId("owner".into()),
                "session",
                "project",
                &GoalSpec {
                    original_intent: "test attempts".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: Default::default(),
                },
            )
            .unwrap();
        (store, tmp, goal.id)
    }

    fn begin(store: &ObjectiveStore, goal_id: GoalId, sequence: u32) -> GoalAttempt {
        store
            .begin_attempt(
                goal_id,
                sequence,
                &RuntimeId("worker".into()),
                CognitiveRole::Worker,
                &serde_json::json!({"task": sequence}),
            )
            .unwrap()
    }

    fn usage() -> AttemptUsage {
        AttemptUsage {
            input_tokens: 3,
            output_tokens: 2,
            cost_usd: Some(0.1),
            elapsed_ms: 50,
        }
    }

    #[test]
    fn begin_creates_running_attempt_and_event() {
        let (store, _tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        assert_eq!(attempt.status, AttemptStatus::Running);
        assert_eq!(attempt.runtime_id.0, "worker");
        assert_eq!(attempt.input, serde_json::json!({"task": 1}));
        let event: String = store
            .db
            .query_row(
                "SELECT event_type FROM goal_events WHERE objective_id = ?1 ORDER BY version DESC LIMIT 1",
                params![goal_id.0],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event, "attempt_started");
    }

    #[test]
    fn finish_success_persists_output_evidence_and_usage() {
        let (store, _tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        let finished = store
            .finish_attempt(
                attempt.id,
                Ok(RuntimeResult {
                    output: "done".into(),
                    usage: usage(),
                    evidence: vec![AttemptEvidence {
                        kind: "test".into(),
                        summary: "passed".into(),
                        content: "ok".into(),
                    }],
                }),
            )
            .unwrap();
        assert_eq!(finished.status, AttemptStatus::Succeeded);
        assert_eq!(finished.output.unwrap().output, "done");
        assert_eq!(finished.usage, usage());
        assert_eq!(finished.evidence.len(), 1);
        assert!(finished.ended_at.is_some());
    }

    #[test]
    fn finish_failure_persists_classification() {
        let (store, _tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        let finished = store
            .finish_attempt(
                attempt.id,
                Err(RuntimeFailure {
                    class: FailureClass::Compilation,
                    message: "failed".into(),
                    retryable: true,
                    usage: usage(),
                    evidence: vec![],
                }),
            )
            .unwrap();
        assert_eq!(finished.status, AttemptStatus::Failed);
        assert_eq!(finished.failure.unwrap().class, FailureClass::Compilation);
    }

    #[test]
    fn cancel_requires_cancel_class_and_is_terminal() {
        let (store, _tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        let cancelled = store
            .cancel_attempt(
                attempt.id,
                RuntimeFailure {
                    class: FailureClass::Cancelled,
                    message: "owner cancelled".into(),
                    retryable: false,
                    usage: usage(),
                    evidence: vec![],
                },
            )
            .unwrap();
        assert_eq!(cancelled.status, AttemptStatus::Cancelled);
        assert!(store
            .finish_attempt(cancelled.id, Ok(RuntimeResult::default()))
            .is_err());
    }

    #[test]
    fn duplicate_sequence_is_rejected() {
        let (store, _tmp, goal_id) = setup();
        begin(&store, goal_id, 1);
        assert!(store
            .begin_attempt(
                goal_id,
                1,
                &RuntimeId("other".into()),
                CognitiveRole::Reviewer,
                &serde_json::json!({}),
            )
            .is_err());
    }

    #[test]
    fn attempt_identity_and_input_are_database_immutable() {
        let (store, _tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        let error = store
            .db
            .execute(
                "UPDATE goal_attempts SET runtime_id = 'changed', input_json = '{}' WHERE attempt_id = ?1",
                params![attempt.id.0.to_string()],
            )
            .unwrap_err();
        assert!(error.to_string().contains("immutable"));
    }

    #[test]
    fn attempts_are_listed_newest_first() {
        let (store, _tmp, goal_id) = setup();
        begin(&store, goal_id, 1);
        begin(&store, goal_id, 2);
        begin(&store, goal_id, 3);
        let attempts = store.attempts_for_goal(goal_id, 2).unwrap();
        assert_eq!(
            attempts.iter().map(|a| a.sequence).collect::<Vec<_>>(),
            [3, 2]
        );
    }

    #[test]
    fn reopen_recovery_cancels_stale_running_without_rerun() {
        let (store, tmp, goal_id) = setup();
        let attempt = begin(&store, goal_id, 1);
        drop(store);

        let reopened = ObjectiveStore::open(tmp.path()).unwrap();
        let recovered = reopened.recover_stale_attempts().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, attempt.id);
        assert_eq!(recovered[0].status, AttemptStatus::Cancelled);
        assert_eq!(
            recovered[0].failure.as_ref().unwrap().class,
            FailureClass::Cancelled
        );
        assert!(reopened.recover_stale_attempts().unwrap().is_empty());
    }
}
