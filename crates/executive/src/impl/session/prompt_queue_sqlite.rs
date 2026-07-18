//! Durable G3 prompt queue store.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::{PrincipalId, PromptEnvelope, PromptId, PromptState, ThreadId};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::service::session_input::PromptQueueStore;

pub struct SqlitePromptQueueStore {
    connection: Mutex<Connection>,
}

impl SqlitePromptQueueStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).context("open prompt queue sqlite")?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS prompt_queue (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 prompt_id TEXT NOT NULL UNIQUE,
                 principal_id TEXT NOT NULL,
                 thread_id TEXT NOT NULL,
                 idempotency_key TEXT NOT NULL,
                 envelope_json TEXT NOT NULL,
                 consume_receipt TEXT,
                 UNIQUE(principal_id, thread_id, idempotency_key)
             );
             CREATE INDEX IF NOT EXISTS prompt_queue_owner_order
               ON prompt_queue(principal_id, thread_id, sequence);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[async_trait]
impl PromptQueueStore for SqlitePromptQueueStore {
    async fn append(&self, envelope: PromptEnvelope) -> Result<PromptEnvelope> {
        let json = serde_json::to_string(&envelope)?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT OR IGNORE INTO prompt_queue
             (prompt_id, principal_id, thread_id, idempotency_key, envelope_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                envelope.prompt_id.0.to_string(),
                envelope.principal_id.0,
                envelope.thread_id.0,
                envelope.idempotency_key,
                json,
            ],
        )?;
        let stored: String = transaction.query_row(
            "SELECT envelope_json FROM prompt_queue
             WHERE principal_id = ?1 AND thread_id = ?2 AND idempotency_key = ?3",
            params![
                envelope.principal_id.0,
                envelope.thread_id.0,
                envelope.idempotency_key
            ],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok(serde_json::from_str(&stored)?)
    }

    async fn get(&self, id: PromptId) -> Result<Option<PromptEnvelope>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let json: Option<String> = connection
            .query_row(
                "SELECT envelope_json FROM prompt_queue WHERE prompt_id = ?1",
                params![id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    async fn update(&self, envelope: PromptEnvelope) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = connection.execute(
            "UPDATE prompt_queue SET envelope_json = ?1 WHERE prompt_id = ?2",
            params![
                serde_json::to_string(&envelope)?,
                envelope.prompt_id.0.to_string()
            ],
        )?;
        anyhow::ensure!(changed == 1, "prompt not found");
        Ok(())
    }

    async fn ordered(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<Vec<PromptEnvelope>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut statement = connection.prepare(
            "SELECT envelope_json FROM prompt_queue
             WHERE principal_id = ?1 AND thread_id = ?2 ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![principal.0, thread.0], |row| {
            row.get::<_, String>(0)
        })?;
        let mut envelopes = Vec::new();
        for row in rows {
            envelopes.push(serde_json::from_str(&row?)?);
        }
        Ok(envelopes)
    }

    async fn mark_consumed(&self, id: PromptId, receipt: &str) -> Result<bool> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(String, Option<String>)> = transaction
            .query_row(
                "SELECT envelope_json, consume_receipt FROM prompt_queue WHERE prompt_id = ?1",
                params![id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((json, existing_receipt)) = row else {
            anyhow::bail!("prompt not found");
        };
        if existing_receipt.is_some() {
            transaction.commit()?;
            return Ok(false);
        }
        let mut envelope: PromptEnvelope = serde_json::from_str(&json)?;
        envelope.state = PromptState::Completed;
        envelope.version = envelope.version.saturating_add(1);
        transaction.execute(
            "UPDATE prompt_queue SET envelope_json = ?1, consume_receipt = ?2
             WHERE prompt_id = ?3 AND consume_receipt IS NULL",
            params![serde_json::to_string(&envelope)?, receipt, id.0.to_string()],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::session_input::SessionInputCoordinator;
    use fabric::{ConnectionId, PromptKind};
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn reopen_preserves_queue_order_running_state_and_consumption() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prompt-queue.sqlite");
        let principal = PrincipalId("p".into());
        let thread = ThreadId("t".into());
        let first_id;
        let interjection_id;
        {
            let store = Arc::new(SqlitePromptQueueStore::open(&path).unwrap());
            let coordinator = SessionInputCoordinator::new(store);
            let first = coordinator
                .enqueue(
                    principal.clone(),
                    ConnectionId(Uuid::nil()),
                    thread.clone(),
                    PromptKind::Prompt,
                    "first".into(),
                    "i1".into(),
                )
                .await
                .unwrap();
            first_id = first.prompt_id;
            coordinator
                .enqueue(
                    principal.clone(),
                    ConnectionId(Uuid::nil()),
                    thread.clone(),
                    PromptKind::Prompt,
                    "second".into(),
                    "i2".into(),
                )
                .await
                .unwrap();
            let interjection = coordinator
                .enqueue(
                    principal.clone(),
                    ConnectionId(Uuid::nil()),
                    thread.clone(),
                    PromptKind::Interjection,
                    "aside".into(),
                    "i3".into(),
                )
                .await
                .unwrap();
            interjection_id = interjection.prompt_id;
            assert_eq!(
                coordinator
                    .take_next(&principal, &thread)
                    .await
                    .unwrap()
                    .unwrap()
                    .prompt_id,
                first_id
            );
            assert_eq!(
                coordinator
                    .drain_interjections_at_safe_point(&principal, &thread, "receipt")
                    .await
                    .unwrap(),
                ["aside"]
            );
        }

        let store = Arc::new(SqlitePromptQueueStore::open(&path).unwrap());
        let coordinator = SessionInputCoordinator::new(store.clone());
        let snapshot = coordinator.snapshot(&principal, &thread).await.unwrap();
        assert_eq!(snapshot.running, Some(first_id));
        assert_eq!(snapshot.pending.len(), 1);
        assert_eq!(snapshot.pending[0].content, "second");
        assert_eq!(
            store.get(interjection_id).await.unwrap().unwrap().state,
            PromptState::Completed
        );
        assert!(!store
            .mark_consumed(interjection_id, "replay")
            .await
            .unwrap());
    }
}
