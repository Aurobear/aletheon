use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use fabric::types::agent_control::MAX_LIST_ITEMS;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentHandle, AgentId, AgentProfileId, AgentResult,
    AgentRunStatus, AgentSnapshot, AgentSpawnRequest, OperationId, ProcessId, RuntimeId,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::repository::{AgentMessageRecord, AgentRunRecord, AgentRunRepository};

const MIGRATION: &str = include_str!("migrations/001_agent_runs.sql");
const RUN_COLUMNS: &str = "agent_id, root_agent_id, parent_agent_id, process_id, operation_id, \
runtime_id, profile_id, status, request_json, request_hash, result_json, created_at_ms, \
started_at_ms, ended_at_ms, last_error, version, retain_until_ms";

#[derive(Clone)]
pub struct SqliteAgentRunRepository {
    connection: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for SqliteAgentRunRepository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteAgentRunRepository")
            .finish_non_exhaustive()
    }
}

impl SqliteAgentRunRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AgentControlError> {
        let connection = Connection::open(path).map_err(persistence)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, AgentControlError> {
        let connection = Connection::open_in_memory().map_err(persistence)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, AgentControlError> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(persistence)?;
        connection.execute_batch(MIGRATION).map_err(persistence)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn request_hash(request: &AgentSpawnRequest) -> Result<String, AgentControlError> {
        request.validate()?;
        let encoded = serde_json::to_vec(request).map_err(persistence)?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

#[async_trait]
impl AgentRunRepository for SqliteAgentRunRepository {
    async fn create(&self, run: &AgentRunRecord) -> Result<(), AgentControlError> {
        run.request.validate()?;
        if run.request_hash != Self::request_hash(&run.request)? {
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent request hash does not match request payload",
            ));
        }
        if run.snapshot.handle.agent_id != run.request.root_agent_id
            && run.snapshot.handle.parent_agent_id.is_none()
        {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                "non-root Agent requires a parent Agent",
            ));
        }
        if run.snapshot.status != AgentRunStatus::Queued || run.version != 0 {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                "new Agent run must be queued at version zero",
            ));
        }

        let request_json = serde_json::to_string(&run.request).map_err(persistence)?;
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(persistence)?;
        let inserted = transaction.execute(
            "INSERT INTO agent_runs (
                agent_id, root_agent_id, parent_agent_id, process_id, operation_id,
                runtime_id, profile_id, status, request_json, request_hash, result_json,
                created_at_ms, started_at_ms, ended_at_ms, last_error, version, retain_until_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, NULL, NULL, NULL, 0, ?12)",
            params![
                run.snapshot.handle.agent_id.0.to_string(),
                run.snapshot.handle.root_agent_id.0.to_string(),
                run.snapshot.handle.parent_agent_id.map(|id| id.0.to_string()),
                run.snapshot.handle.process_id.0.to_string(),
                run.snapshot.handle.operation_id.0.to_string(),
                run.snapshot.handle.runtime_id.0,
                run.snapshot.handle.profile_id.0,
                status_wire(run.snapshot.status),
                request_json,
                run.request_hash,
                run.snapshot.created_at_ms,
                run.retain_until_ms,
            ],
        );
        match inserted {
            Ok(1) => transaction.commit().map_err(persistence),
            Ok(_) => Err(persistence("Agent run insert affected no row")),
            Err(error)
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                Err(control_error(
                    AgentControlErrorKind::Conflict,
                    "Agent identity or lifecycle resource already exists",
                ))
            }
            Err(error) => Err(persistence(error)),
        }
    }

    async fn transition(
        &self,
        agent: AgentId,
        expected: AgentRunStatus,
        next: AgentRunStatus,
        result: Option<AgentResult>,
        error: Option<String>,
        now_ms: i64,
    ) -> Result<AgentRunRecord, AgentControlError> {
        if !can_transition(expected, next) {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                format!("illegal Agent transition {expected:?} -> {next:?}"),
            ));
        }
        if let Some(value) = &result {
            value.validate()?;
        }
        if next == AgentRunStatus::Succeeded && result.is_none() {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                "successful Agent transition requires a result",
            ));
        }
        let result_json = result
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(persistence)?;
        let started_at = (next == AgentRunStatus::Running).then_some(now_ms);
        let ended_at = next.is_terminal().then_some(now_ms);
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(persistence)?;
        let changed = transaction
            .execute(
                "UPDATE agent_runs SET
                    status = ?1,
                    result_json = COALESCE(?2, result_json),
                    started_at_ms = COALESCE(started_at_ms, ?3),
                    ended_at_ms = COALESCE(ended_at_ms, ?4),
                    last_error = ?5,
                    version = version + 1
                 WHERE agent_id = ?6 AND status = ?7",
                params![
                    status_wire(next),
                    result_json,
                    started_at,
                    ended_at,
                    error.map(|value| bound_text(&value, 16 * 1024)),
                    agent.0.to_string(),
                    status_wire(expected),
                ],
            )
            .map_err(persistence)?;
        if changed != 1 {
            let current: Option<String> = transaction
                .query_row(
                    "SELECT status FROM agent_runs WHERE agent_id = ?1",
                    [agent.0.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(persistence)?;
            return match current {
                None => Err(control_error(
                    AgentControlErrorKind::NotFound,
                    "Agent run was not found",
                )),
                Some(current) => Err(control_error(
                    AgentControlErrorKind::Conflict,
                    format!("Agent transition expected {expected:?}, stored status is {current}"),
                )),
            };
        }
        let record = query_one(&transaction, agent)?;
        transaction.commit().map_err(persistence)?;
        Ok(record)
    }

    async fn get(&self, agent: AgentId) -> Result<Option<AgentRunRecord>, AgentControlError> {
        let connection = self.connection.lock();
        query_optional(&connection, agent)
    }

    async fn list_root(
        &self,
        root: AgentId,
        status: Option<AgentRunStatus>,
        limit: usize,
    ) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid("Agent list limit is invalid"));
        }
        let connection = self.connection.lock();
        let sql = match status {
            Some(_) => format!(
                "SELECT {RUN_COLUMNS} FROM agent_runs WHERE root_agent_id = ?1 AND status = ?2 ORDER BY created_at_ms DESC, agent_id LIMIT ?3"
            ),
            None => format!(
                "SELECT {RUN_COLUMNS} FROM agent_runs WHERE root_agent_id = ?1 ORDER BY created_at_ms DESC, agent_id LIMIT ?2"
            ),
        };
        let mut statement = connection.prepare(&sql).map_err(persistence)?;
        let rows = match status {
            Some(value) => statement
                .query_map(
                    params![root.0.to_string(), status_wire(value), limit as i64],
                    map_run_row,
                )
                .map_err(persistence)?,
            None => statement
                .query_map(params![root.0.to_string(), limit as i64], map_run_row)
                .map_err(persistence)?,
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(persistence)
    }

    async fn append_message(
        &self,
        agent: AgentId,
        from: AgentId,
        content: &str,
        created_at_ms: i64,
    ) -> Result<AgentMessageRecord, AgentControlError> {
        let content_hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(persistence)?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM agent_runs WHERE agent_id = ?1",
                [agent.0.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(persistence)?;
        let Some(status) = status else {
            return Err(control_error(
                AgentControlErrorKind::NotFound,
                "Agent run was not found",
            ));
        };
        if parse_status_text(&status)?.is_terminal() {
            return Err(control_error(
                AgentControlErrorKind::Terminal,
                "terminal Agent rejects new messages",
            ));
        }
        let sequence: u64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_run_messages WHERE agent_id = ?1",
                [agent.0.to_string()],
                |row| row.get(0),
            )
            .map_err(persistence)?;
        transaction
            .execute(
                "INSERT INTO agent_run_messages(agent_id, sequence, from_agent_id, content_hash, content_bytes, created_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![agent.0.to_string(), sequence, from.0.to_string(), content_hash, content.len() as i64, created_at_ms],
            )
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)?;
        Ok(AgentMessageRecord {
            sequence,
            content_hash,
        })
    }
}

fn query_one(connection: &Connection, agent: AgentId) -> Result<AgentRunRecord, AgentControlError> {
    query_optional(connection, agent)?
        .ok_or_else(|| control_error(AgentControlErrorKind::NotFound, "Agent run was not found"))
}

fn query_optional(
    connection: &Connection,
    agent: AgentId,
) -> Result<Option<AgentRunRecord>, AgentControlError> {
    connection
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM agent_runs WHERE agent_id = ?1"),
            [agent.0.to_string()],
            map_run_row,
        )
        .optional()
        .map_err(persistence)
}

fn map_run_row(row: &Row<'_>) -> rusqlite::Result<AgentRunRecord> {
    map_run_row_fallible(row).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn map_run_row_fallible(row: &Row<'_>) -> Result<AgentRunRecord, AgentControlError> {
    let agent_id = parse_agent(&column::<String>(row, 0)?)?;
    let root_agent_id = parse_agent(&column::<String>(row, 1)?)?;
    let parent_agent_id = column::<Option<String>>(row, 2)?
        .map(|value| parse_agent(&value))
        .transpose()?;
    let process_id = ProcessId(parse_uuid(&column::<String>(row, 3)?)?);
    let operation_id = OperationId(parse_uuid(&column::<String>(row, 4)?)?);
    let runtime_id = RuntimeId(column(row, 5)?);
    let profile_id = AgentProfileId(column(row, 6)?);
    let status = parse_status_text(&column::<String>(row, 7)?)?;
    let request = serde_json::from_str(&column::<String>(row, 8)?).map_err(persistence)?;
    let request_hash = column(row, 9)?;
    let result = column::<Option<String>>(row, 10)?
        .map(|value| serde_json::from_str(&value).map_err(persistence))
        .transpose()?;
    Ok(AgentRunRecord {
        snapshot: AgentSnapshot {
            handle: AgentHandle {
                agent_id,
                root_agent_id,
                parent_agent_id,
                process_id,
                operation_id,
                runtime_id,
                profile_id,
            },
            status,
            result,
            created_at_ms: column(row, 11)?,
            started_at_ms: column(row, 12)?,
            ended_at_ms: column(row, 13)?,
            last_error: column(row, 14)?,
        },
        request,
        request_hash,
        version: column(row, 15)?,
        retain_until_ms: column(row, 16)?,
    })
}

fn column<T: rusqlite::types::FromSql>(
    row: &Row<'_>,
    index: usize,
) -> Result<T, AgentControlError> {
    row.get(index).map_err(persistence)
}

fn can_transition(from: AgentRunStatus, to: AgentRunStatus) -> bool {
    use AgentRunStatus::{Cancelled, Failed, Interrupted, Queued, Running, Succeeded, Waiting};
    matches!(
        (from, to),
        (Queued, Running)
            | (Queued, Cancelled)
            | (Queued, Failed)
            | (Queued, Interrupted)
            | (Running, Waiting)
            | (Waiting, Running)
            | (Running, Succeeded)
            | (Running, Failed)
            | (Running, Cancelled)
            | (Running, Interrupted)
            | (Waiting, Succeeded)
            | (Waiting, Failed)
            | (Waiting, Cancelled)
            | (Waiting, Interrupted)
    )
}

fn status_wire(status: AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Queued => "queued",
        AgentRunStatus::Running => "running",
        AgentRunStatus::Waiting => "waiting",
        AgentRunStatus::Succeeded => "succeeded",
        AgentRunStatus::Failed => "failed",
        AgentRunStatus::Cancelled => "cancelled",
        AgentRunStatus::Interrupted => "interrupted",
    }
}

fn parse_status_text(value: &str) -> Result<AgentRunStatus, AgentControlError> {
    match value {
        "queued" => Ok(AgentRunStatus::Queued),
        "running" => Ok(AgentRunStatus::Running),
        "waiting" => Ok(AgentRunStatus::Waiting),
        "succeeded" => Ok(AgentRunStatus::Succeeded),
        "failed" => Ok(AgentRunStatus::Failed),
        "cancelled" => Ok(AgentRunStatus::Cancelled),
        "interrupted" => Ok(AgentRunStatus::Interrupted),
        _ => Err(control_error(
            AgentControlErrorKind::Persistence,
            format!("unknown persisted Agent status: {value}"),
        )),
    }
}

fn parse_agent(value: &str) -> Result<AgentId, AgentControlError> {
    parse_uuid(value).map(AgentId)
}

fn parse_uuid(value: &str) -> Result<Uuid, AgentControlError> {
    Uuid::parse_str(value).map_err(persistence)
}

fn bound_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn control_error(kind: AgentControlErrorKind, message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind,
        message: message.into(),
    }
}

fn persistence(error: impl std::fmt::Display) -> AgentControlError {
    control_error(AgentControlErrorKind::Persistence, error.to_string())
}
