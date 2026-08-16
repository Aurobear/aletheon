//! Application contracts for one-shot, non-interactive execution.

use std::path::Path;

use anyhow::Context;
use gateway::protocol::exec::ExecEventEnvelope;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

const STORE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecClaim {
    Acquired,
    Replay(Box<ExecEventEnvelope>),
    InProgress,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecIdempotencyError {
    #[error("exec idempotency key is already bound to a different request")]
    RequestConflict,
    #[error("exec idempotency store failure: {0}")]
    Store(#[from] anyhow::Error),
}

pub struct ExecIdempotencyStore {
    connection: Mutex<Connection>,
}

impl ExecIdempotencyStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS exec_idempotency (
                 schema_version INTEGER NOT NULL,
                 principal_id TEXT NOT NULL,
                 idempotency_key TEXT NOT NULL,
                 request_digest TEXT NOT NULL,
                 state TEXT NOT NULL CHECK(state IN ('in_progress','terminal')),
                 terminal_json TEXT,
                 PRIMARY KEY(principal_id, idempotency_key)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub async fn claim(
        &self,
        principal_id: &str,
        idempotency_key: &str,
        request_digest: &str,
    ) -> Result<ExecClaim, ExecIdempotencyError> {
        let connection = self.connection.lock().await;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO exec_idempotency
                 (schema_version, principal_id, idempotency_key, request_digest, state)
                 VALUES (?1, ?2, ?3, ?4, 'in_progress')",
                params![
                    STORE_SCHEMA_VERSION,
                    principal_id,
                    idempotency_key,
                    request_digest
                ],
            )
            .context("claiming exec idempotency key")?;
        if inserted == 1 {
            return Ok(ExecClaim::Acquired);
        }
        let row: Option<(String, String, Option<String>)> = connection
            .query_row(
                "SELECT request_digest, state, terminal_json
                 FROM exec_idempotency
                 WHERE principal_id=?1 AND idempotency_key=?2",
                params![principal_id, idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .context("reading exec idempotency claim")?;
        let Some((existing_digest, state, terminal_json)) = row else {
            return Err(ExecIdempotencyError::Store(anyhow::anyhow!(
                "idempotency row disappeared after conflict"
            )));
        };
        if existing_digest != request_digest {
            return Err(ExecIdempotencyError::RequestConflict);
        }
        match (state.as_str(), terminal_json) {
            ("terminal", Some(json)) => Ok(ExecClaim::Replay(Box::new(
                serde_json::from_str(&json).context("decoding terminal exec receipt")?,
            ))),
            ("in_progress", _) => Ok(ExecClaim::InProgress),
            _ => Err(ExecIdempotencyError::Store(anyhow::anyhow!(
                "invalid exec idempotency state"
            ))),
        }
    }

    pub async fn complete(
        &self,
        principal_id: &str,
        idempotency_key: &str,
        request_digest: &str,
        terminal: &ExecEventEnvelope,
    ) -> Result<(), ExecIdempotencyError> {
        let terminal_json = serde_json::to_string(terminal)
            .context("encoding terminal exec idempotency receipt")?;
        let connection = self.connection.lock().await;
        let changed = connection
            .execute(
                "UPDATE exec_idempotency
                 SET state='terminal', terminal_json=?4
                 WHERE principal_id=?1 AND idempotency_key=?2
                   AND request_digest=?3 AND state='in_progress'",
                params![principal_id, idempotency_key, request_digest, terminal_json],
            )
            .context("completing exec idempotency receipt")?;
        if changed != 1 {
            return Err(ExecIdempotencyError::RequestConflict);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{OperationId, TurnMetrics};
    use gateway::protocol::exec::{ExecEvent, ExecTerminalKind};

    fn terminal() -> ExecEventEnvelope {
        ExecEventEnvelope::v1(
            2,
            "session",
            "task",
            "turn",
            None,
            OperationId::new(),
            ExecEvent::Terminal {
                status: ExecTerminalKind::Completed,
                output: "done".into(),
                metrics: TurnMetrics::default(),
                error_code: None,
            },
        )
    }

    #[tokio::test]
    async fn terminal_receipt_replays_and_unknown_execution_is_not_repeated() {
        let temp = tempfile::tempdir().unwrap();
        let store = ExecIdempotencyStore::open(&temp.path().join("exec.db")).unwrap();
        assert_eq!(
            store.claim("p", "key", "digest").await.unwrap(),
            ExecClaim::Acquired
        );
        assert_eq!(
            store.claim("p", "key", "digest").await.unwrap(),
            ExecClaim::InProgress
        );
        let receipt = terminal();
        store
            .complete("p", "key", "digest", &receipt)
            .await
            .unwrap();
        assert_eq!(
            store.claim("p", "key", "digest").await.unwrap(),
            ExecClaim::Replay(Box::new(receipt))
        );
        assert!(matches!(
            store.claim("p", "key", "other").await,
            Err(ExecIdempotencyError::RequestConflict)
        ));
    }
}
