use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use fabric::types::agent_control::MAX_LIST_ITEMS;
use fabric::{
    AgentBroadcastRef, AgentControlError, AgentControlErrorKind, AgentHandle, AgentId,
    AgentMessageDeliveryState, AgentMessagePayload, AgentProfileId, AgentRecoveryReceipt,
    AgentResult, AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AgoraSpaceId, OperationId,
    ProcessId, RuntimeId, RuntimeResumability,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::repository::{
    AgentMessageRecord, AgentResourceLease, AgentResourceLeaseKind, AgentRunRecord,
    AgentRunRepository,
};

const MIGRATION: &str = include_str!("migrations/001_agent_runs.sql");
const MESSAGE_MIGRATION: &str = include_str!("migrations/002_agent_messages.sql");
const RECOVERY_MIGRATION: &str = include_str!("migrations/003_agent_recovery.sql");
const RUN_COLUMNS: &str = "agent_id, root_agent_id, parent_agent_id, process_id, operation_id, \
    runtime_id, profile_id, status, request_json, request_hash, result_json, created_at_ms, \
    started_at_ms, ended_at_ms, last_error, version, retain_until_ms, workspace_id, \
    root_process_id, broadcast_refs_json, resumability_json, recovery_json";

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
        connection
            .execute_batch(MESSAGE_MIGRATION)
            .map_err(persistence)?;
        connection
            .execute_batch(RECOVERY_MIGRATION)
            .map_err(persistence)?;
        ensure_workspace_columns(&connection)?;
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
        if run.workspace_id != super::repository::agent_workspace_id(run.agent_id()) {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                "Agent workspace ID does not match durable Agent identity",
            ));
        }
        if run.broadcast_refs != run.request.broadcast_refs {
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent broadcast receipts do not match spawn request",
            ));
        }

        let request_json = serde_json::to_string(&run.request).map_err(persistence)?;
        let broadcast_refs_json =
            serde_json::to_string(&run.broadcast_refs).map_err(persistence)?;
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(persistence)?;
        let inserted = transaction.execute(
            "INSERT INTO agent_runs (
                agent_id, root_agent_id, parent_agent_id, process_id, operation_id,
                runtime_id, profile_id, status, request_json, request_hash, result_json,
                created_at_ms, started_at_ms, ended_at_ms, last_error, version, retain_until_ms,
                workspace_id, root_process_id, broadcast_refs_json, resumability_json, recovery_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, NULL, NULL, NULL, 0, ?12, ?13, ?14, ?15, ?16, NULL)",
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
                run.workspace_id.0,
                run.root_process_id.0.to_string(),
                broadcast_refs_json,
                serde_json::to_string(&run.resumability).map_err(persistence)?,
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

    async fn list_open(&self, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid(
                "Agent recovery limit is invalid",
            ));
        }
        let connection = self.connection.lock();
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE status IN ('queued','running','waiting') ORDER BY created_at_ms,agent_id LIMIT ?1"
        );
        let mut statement = connection.prepare(&sql).map_err(persistence)?;
        let rows = statement
            .query_map([limit as i64], map_run_row)
            .map_err(persistence)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(persistence)
    }

    async fn list_recent(&self, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid(
                "Agent recent-run limit is invalid",
            ));
        }
        let connection = self.connection.lock();
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs ORDER BY created_at_ms DESC,agent_id LIMIT ?1"
        );
        let mut statement = connection.prepare(&sql).map_err(persistence)?;
        let rows = statement
            .query_map([limit as i64], map_run_row)
            .map_err(persistence)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(persistence)
    }

    async fn record_recovery(
        &self,
        agent: AgentId,
        receipt: &AgentRecoveryReceipt,
    ) -> Result<AgentRunRecord, AgentControlError> {
        if receipt.daemon_generation.trim().is_empty() || receipt.idempotency_key.trim().is_empty()
        {
            return Err(AgentControlError::invalid(
                "Agent recovery receipt is incomplete",
            ));
        }
        let json = serde_json::to_string(receipt).map_err(persistence)?;
        let connection = self.connection.lock();
        let existing: Option<String> = connection
            .query_row(
                "SELECT recovery_json FROM agent_runs WHERE agent_id=?1",
                [agent.0.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(persistence)?
            .flatten();
        if let Some(existing) = existing {
            let stored: AgentRecoveryReceipt =
                serde_json::from_str(&existing).map_err(persistence)?;
            if stored == *receipt {
                return query_one(&connection, agent);
            }
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent run already has a different recovery decision",
            ));
        }
        let changed = connection
            .execute(
                "UPDATE agent_runs SET recovery_json=?1,version=version+1 WHERE agent_id=?2",
                params![json, agent.0.to_string()],
            )
            .map_err(persistence)?;
        if changed != 1 {
            return Err(control_error(
                AgentControlErrorKind::NotFound,
                "Agent run was not found",
            ));
        }
        query_one(&connection, agent)
    }

    async fn compact_terminal(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentId>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid(
                "Agent compaction limit is invalid",
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(persistence)?;
        let ids = {
            let mut statement = transaction
                .prepare("SELECT agent_id FROM agent_runs WHERE status IN ('succeeded','failed','cancelled','interrupted') AND retain_until_ms<=?1 AND NOT EXISTS (SELECT 1 FROM agent_resource_leases l WHERE l.agent_id=agent_runs.agent_id) ORDER BY retain_until_ms,agent_id LIMIT ?2")
                .map_err(persistence)?;
            let collected = statement
                .query_map(params![now_ms, limit as i64], |row| row.get::<_, String>(0))
                .map_err(persistence)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(persistence)?;
            collected
        };
        let mut removed = Vec::new();
        for value in ids {
            let id = parse_agent(&value)?;
            transaction
                .execute("DELETE FROM agent_messages_v2 WHERE agent_id=?1", [&value])
                .map_err(persistence)?;
            transaction
                .execute("DELETE FROM agent_run_messages WHERE agent_id=?1", [&value])
                .map_err(persistence)?;
            transaction
                .execute("DELETE FROM agent_runs WHERE agent_id=?1", [&value])
                .map_err(persistence)?;
            removed.push(id);
        }
        transaction.commit().map_err(persistence)?;
        Ok(removed)
    }

    async fn put_resource_lease(
        &self,
        lease: &AgentResourceLease,
    ) -> Result<(), AgentControlError> {
        if lease.lease_key.trim().is_empty()
            || lease.owner.trim().is_empty()
            || lease.expires_at_ms <= 0
        {
            return Err(AgentControlError::invalid(
                "Agent resource lease is incomplete",
            ));
        }
        if lease.kind == AgentResourceLeaseKind::Worktree
            && (lease.worktree_root.as_deref().is_none_or(str::is_empty)
                || lease.worktree_path.as_deref().is_none_or(str::is_empty)
                || lease.expected_head.as_deref().is_none_or(str::is_empty))
        {
            return Err(AgentControlError::invalid(
                "worktree lease verification data is incomplete",
            ));
        }
        let connection = self.connection.lock();
        let changed = connection
            .execute(
                "INSERT INTO agent_resource_leases(lease_key,agent_id,kind,owner,expires_at_ms,worktree_root,worktree_path,expected_head) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(lease_key) DO NOTHING",
                params![lease.lease_key, lease.agent_id.0.to_string(), lease_kind_wire(lease.kind), lease.owner, lease.expires_at_ms, lease.worktree_root, lease.worktree_path, lease.expected_head],
            )
            .map_err(persistence)?;
        if changed == 0 {
            let existing = query_resource_lease(&connection, &lease.lease_key)?;
            if existing.as_ref() != Some(lease) {
                return Err(control_error(
                    AgentControlErrorKind::Conflict,
                    "resource lease key conflicts with different metadata",
                ));
            }
        }
        Ok(())
    }

    async fn list_expired_resource_leases(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<AgentResourceLease>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid(
                "resource lease recovery limit is invalid",
            ));
        }
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare("SELECT lease_key,agent_id,kind,owner,expires_at_ms,worktree_root,worktree_path,expected_head FROM agent_resource_leases WHERE expires_at_ms<=?1 ORDER BY expires_at_ms,lease_key LIMIT ?2")
            .map_err(persistence)?;
        let rows = statement
            .query_map(params![now_ms, limit as i64], map_resource_lease)
            .map_err(persistence)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(persistence)
    }

    async fn list_agent_resource_leases(
        &self,
        agent: AgentId,
        limit: usize,
    ) -> Result<Vec<AgentResourceLease>, AgentControlError> {
        if limit == 0 || limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid(
                "resource lease recovery limit is invalid",
            ));
        }
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare(
                "SELECT lease_key, agent_id, kind, owner, expires_at_ms,
                        worktree_root, worktree_path, expected_head
                 FROM agent_resource_leases WHERE agent_id=?1
                 ORDER BY lease_key ASC LIMIT ?2",
            )
            .map_err(persistence)?;
        let rows = statement
            .query_map(
                rusqlite::params![agent.0.to_string(), limit as i64],
                map_resource_lease,
            )
            .map_err(persistence)?;
        rows.map(|row| row.map_err(persistence)).collect()
    }

    async fn delete_resource_lease(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError> {
        let connection = self.connection.lock();
        Ok(connection
            .execute(
                "DELETE FROM agent_resource_leases WHERE lease_key=?1 AND owner=?2",
                params![lease_key, expected_owner],
            )
            .map_err(persistence)?
            == 1)
    }

    async fn append_message(
        &self,
        agent: AgentId,
        from: AgentId,
        delivery_id: Uuid,
        payload: &AgentMessagePayload,
        created_at_ms: i64,
    ) -> Result<AgentMessageRecord, AgentControlError> {
        payload.validate()?;
        if delivery_id.is_nil() {
            return Err(AgentControlError::invalid(
                "message delivery ID must not be nil",
            ));
        }
        let payload_json = serde_json::to_string(payload).map_err(persistence)?;
        let payload_ref = format!("sha256:{:x}", Sha256::digest(payload_json.as_bytes()));
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
        if let Some(existing) = query_message(&transaction, agent, delivery_id)? {
            transaction.commit().map_err(persistence)?;
            return Ok(existing);
        }
        let sequence: u64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_messages_v2 WHERE agent_id = ?1",
                [agent.0.to_string()],
                |row| row.get(0),
            )
            .map_err(persistence)?;
        transaction
            .execute(
                "INSERT INTO agent_messages_v2(agent_id, delivery_id, sequence, from_agent_id, kind, payload_ref, payload_json, delivery_state, created_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
                params![agent.0.to_string(), delivery_id.to_string(), sequence, from.0.to_string(), message_kind_wire(&payload.kind), payload_ref, payload_json, created_at_ms],
            )
            .map_err(persistence)?;
        transaction.commit().map_err(persistence)?;
        Ok(AgentMessageRecord {
            delivery_id,
            sequence,
            from,
            payload_ref,
            payload: payload.clone(),
            delivery: AgentMessageDeliveryState::Pending,
            created_at_ms,
        })
    }

    async fn mark_message_delivery(
        &self,
        agent: AgentId,
        delivery_id: Uuid,
        delivery: AgentMessageDeliveryState,
    ) -> Result<AgentMessageRecord, AgentControlError> {
        if delivery == AgentMessageDeliveryState::Pending {
            return Err(AgentControlError::invalid(
                "delivery settlement must be terminal",
            ));
        }
        let connection = self.connection.lock();
        let changed = connection
            .execute(
                "UPDATE agent_messages_v2 SET delivery_state = ?1 WHERE agent_id = ?2 AND delivery_id = ?3 AND delivery_state = 'pending'",
                params![delivery_wire(delivery), agent.0.to_string(), delivery_id.to_string()],
            )
            .map_err(persistence)?;
        let record = query_message(&connection, agent, delivery_id)?.ok_or_else(|| {
            control_error(
                AgentControlErrorKind::NotFound,
                "Agent message was not found",
            )
        })?;
        if changed == 0 && record.delivery != delivery {
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent message delivery is already settled differently",
            ));
        }
        Ok(record)
    }
}

fn query_message(
    connection: &Connection,
    agent: AgentId,
    delivery_id: Uuid,
) -> Result<Option<AgentMessageRecord>, AgentControlError> {
    connection
        .query_row(
            "SELECT sequence, from_agent_id, payload_ref, payload_json, delivery_state, created_at_ms FROM agent_messages_v2 WHERE agent_id = ?1 AND delivery_id = ?2",
            params![agent.0.to_string(), delivery_id.to_string()],
            |row| {
                let from: String = row.get(1)?;
                let payload_json: String = row.get(3)?;
                let state: String = row.get(4)?;
                Ok((row.get::<_, u64>(0)?, from, row.get::<_, String>(2)?, payload_json, state, row.get::<_, i64>(5)?))
            },
        )
        .optional()
        .map_err(persistence)?
        .map(|(sequence, from, payload_ref, payload_json, state, created_at_ms)| {
            Ok(AgentMessageRecord {
                delivery_id,
                sequence,
                from: parse_agent(&from)?,
                payload_ref,
                payload: serde_json::from_str(&payload_json).map_err(persistence)?,
                delivery: parse_delivery(&state)?,
                created_at_ms,
            })
        })
        .transpose()
}

fn message_kind_wire(kind: &fabric::AgentMessageKind) -> &'static str {
    use fabric::AgentMessageKind::*;
    match kind {
        Input => "input",
        Progress => "progress",
        Result => "result",
        Signal => "signal",
        Request => "request",
        Response => "response",
    }
}

fn delivery_wire(delivery: AgentMessageDeliveryState) -> &'static str {
    match delivery {
        AgentMessageDeliveryState::Pending => "pending",
        AgentMessageDeliveryState::Delivered => "delivered",
        AgentMessageDeliveryState::Rejected => "rejected",
    }
}

fn parse_delivery(value: &str) -> Result<AgentMessageDeliveryState, AgentControlError> {
    match value {
        "pending" => Ok(AgentMessageDeliveryState::Pending),
        "delivered" => Ok(AgentMessageDeliveryState::Delivered),
        "rejected" => Ok(AgentMessageDeliveryState::Rejected),
        _ => Err(control_error(
            AgentControlErrorKind::Persistence,
            "invalid Agent message delivery state",
        )),
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
        workspace_id: AgoraSpaceId(column(row, 17)?),
        root_process_id: ProcessId(parse_uuid(&column::<String>(row, 18)?)?),
        broadcast_refs: serde_json::from_str::<Vec<AgentBroadcastRef>>(&column::<String>(row, 19)?)
            .map_err(persistence)?,
        version: column(row, 15)?,
        retain_until_ms: column(row, 16)?,
        resumability: serde_json::from_str::<RuntimeResumability>(&column::<String>(row, 20)?)
            .map_err(persistence)?,
        recovery: column::<Option<String>>(row, 21)?
            .map(|value| serde_json::from_str(&value).map_err(persistence))
            .transpose()?,
    })
}

fn ensure_workspace_columns(connection: &Connection) -> Result<(), AgentControlError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(agent_runs)")
        .map_err(persistence)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(persistence)?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(persistence)?;
    drop(statement);

    if !columns.contains("workspace_id") {
        connection
            .execute_batch(
                "ALTER TABLE agent_runs ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '';",
            )
            .map_err(persistence)?;
    }
    if !columns.contains("root_process_id") {
        connection
            .execute_batch(
                "ALTER TABLE agent_runs ADD COLUMN root_process_id TEXT NOT NULL DEFAULT '';",
            )
            .map_err(persistence)?;
    }
    if !columns.contains("broadcast_refs_json") {
        connection
            .execute_batch(
                "ALTER TABLE agent_runs ADD COLUMN broadcast_refs_json TEXT NOT NULL DEFAULT '[]';",
            )
            .map_err(persistence)?;
    }
    if !columns.contains("resumability_json") {
        connection
            .execute_batch(
                "ALTER TABLE agent_runs ADD COLUMN resumability_json TEXT NOT NULL DEFAULT '{\"mode\":\"never\"}';",
            )
            .map_err(persistence)?;
    }
    if !columns.contains("recovery_json") {
        connection
            .execute_batch("ALTER TABLE agent_runs ADD COLUMN recovery_json TEXT;")
            .map_err(persistence)?;
    }
    connection
        .execute_batch(
            "UPDATE agent_runs SET workspace_id = 'agent:' || agent_id WHERE workspace_id = '';
             UPDATE agent_runs SET root_process_id = process_id WHERE root_process_id = '';",
        )
        .map_err(persistence)
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

fn lease_kind_wire(kind: AgentResourceLeaseKind) -> &'static str {
    match kind {
        AgentResourceLeaseKind::Admission => "admission",
        AgentResourceLeaseKind::Mailbox => "mailbox",
        AgentResourceLeaseKind::Execution => "execution",
        AgentResourceLeaseKind::Worktree => "worktree",
    }
}

fn parse_lease_kind(value: &str) -> Result<AgentResourceLeaseKind, AgentControlError> {
    match value {
        "admission" => Ok(AgentResourceLeaseKind::Admission),
        "mailbox" => Ok(AgentResourceLeaseKind::Mailbox),
        "execution" => Ok(AgentResourceLeaseKind::Execution),
        "worktree" => Ok(AgentResourceLeaseKind::Worktree),
        _ => Err(control_error(
            AgentControlErrorKind::Persistence,
            "invalid resource lease kind",
        )),
    }
}

fn map_resource_lease(row: &Row<'_>) -> rusqlite::Result<AgentResourceLease> {
    let mapped = (|| -> Result<AgentResourceLease, AgentControlError> {
        Ok(AgentResourceLease {
            lease_key: column(row, 0)?,
            agent_id: parse_agent(&column::<String>(row, 1)?)?,
            kind: parse_lease_kind(&column::<String>(row, 2)?)?,
            owner: column(row, 3)?,
            expires_at_ms: column(row, 4)?,
            worktree_root: column(row, 5)?,
            worktree_path: column(row, 6)?,
            expected_head: column(row, 7)?,
        })
    })();
    mapped.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn query_resource_lease(
    connection: &Connection,
    key: &str,
) -> Result<Option<AgentResourceLease>, AgentControlError> {
    connection
        .query_row("SELECT lease_key,agent_id,kind,owner,expires_at_ms,worktree_root,worktree_path,expected_head FROM agent_resource_leases WHERE lease_key=?1", [key], map_resource_lease)
        .optional()
        .map_err(persistence)
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
