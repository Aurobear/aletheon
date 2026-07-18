//! Durable G4 workspace checkpoint store.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::types::workspace_checkpoint::{
    CheckpointFileEntry, CheckpointFinalizeState, CheckpointId, TurnCheckpoint,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::service::workspace_checkpoint::CheckpointStore;

pub struct SqliteCheckpointStore {
    connection: Mutex<Connection>,
}

impl SqliteCheckpointStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path).context("open workspace checkpoint sqlite")?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS workspace_checkpoints (
               checkpoint_id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL,
               prompt_index INTEGER NOT NULL,
               checkpoint_json TEXT NOT NULL,
               files_json TEXT NOT NULL,
               UNIQUE(session_id, prompt_index)
             );
             CREATE INDEX IF NOT EXISTS workspace_checkpoint_order
               ON workspace_checkpoints(session_id, prompt_index);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn begin(
        &self,
        checkpoint: TurnCheckpoint,
        files: Vec<CheckpointFileEntry>,
    ) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connection.execute(
            "INSERT INTO workspace_checkpoints
             (checkpoint_id, session_id, prompt_index, checkpoint_json, files_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                checkpoint.checkpoint_id.0.to_string(),
                checkpoint.session_id,
                checkpoint.prompt_index,
                serde_json::to_string(&checkpoint)?,
                serde_json::to_string(&files)?,
            ],
        )?;
        Ok(())
    }

    async fn finalize(&self, id: CheckpointId, state: CheckpointFinalizeState) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let json: Option<String> = transaction
            .query_row(
                "SELECT checkpoint_json FROM workspace_checkpoints WHERE checkpoint_id = ?1",
                params![id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let mut checkpoint: TurnCheckpoint =
            serde_json::from_str(&json.ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?)?;
        if checkpoint.finalize_state == CheckpointFinalizeState::Open {
            checkpoint.finalize_state = state;
            transaction.execute(
                "UPDATE workspace_checkpoints SET checkpoint_json = ?1 WHERE checkpoint_id = ?2",
                params![serde_json::to_string(&checkpoint)?, id.0.to_string()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    async fn load(
        &self,
        session: &str,
        prompt_index: u64,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let row: Option<(String, String)> = connection
            .query_row(
                "SELECT checkpoint_json, files_json FROM workspace_checkpoints
                 WHERE session_id = ?1 AND prompt_index = ?2",
                params![session, prompt_index],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(checkpoint, files)| {
            Ok((
                serde_json::from_str(&checkpoint)?,
                serde_json::from_str(&files)?,
            ))
        })
        .transpose()
    }

    async fn load_by_id(
        &self,
        id: CheckpointId,
    ) -> Result<Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let row: Option<(String, String)> = connection
            .query_row(
                "SELECT checkpoint_json, files_json FROM workspace_checkpoints
                 WHERE checkpoint_id = ?1",
                params![id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(checkpoint, files)| {
            Ok((
                serde_json::from_str(&checkpoint)?,
                serde_json::from_str(&files)?,
            ))
        })
        .transpose()
    }

    async fn truncate_after(&self, session: &str, prompt_index: u64) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connection.execute(
            "DELETE FROM workspace_checkpoints WHERE session_id = ?1 AND prompt_index > ?2",
            params![session, prompt_index],
        )?;
        Ok(())
    }

    async fn stored_bytes(&self) -> Result<u64> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut statement = connection.prepare("SELECT files_json FROM workspace_checkpoints")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut total = 0_u64;
        for row in rows {
            let files: Vec<CheckpointFileEntry> = serde_json::from_str(&row?)?;
            total = total.saturating_add(
                files
                    .iter()
                    .filter_map(|entry| entry.content.as_ref())
                    .map(|content| content.len() as u64)
                    .sum::<u64>(),
            );
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::workspace_checkpoint::{FsDomainRef, WorkspaceIdentity};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn checkpoint(index: u64, state: CheckpointFinalizeState) -> TurnCheckpoint {
        TurnCheckpoint {
            checkpoint_id: CheckpointId::new(),
            session_id: "session".into(),
            thread_id: "thread".into(),
            turn_id: format!("turn-{index}"),
            prompt_index: index,
            workspace: WorkspaceIdentity {
                canonical_path: PathBuf::from("/workspace"),
                repo_fingerprint: None,
            },
            fs_domain: FsDomainRef {
                batch_id: Uuid::new_v4(),
                file_count: 1,
            },
            vcs_domain_ref: None,
            patch_domain_ref: None,
            runtime_checkpoint_ref: None,
            created_at_ms: 1,
            schema_version: 1,
            finalize_state: state,
        }
    }

    #[tokio::test]
    async fn reopen_preserves_state_and_truncate_only_removes_future() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("checkpoints.sqlite");
        let first = checkpoint(1, CheckpointFinalizeState::Open);
        let aborted = checkpoint(2, CheckpointFinalizeState::Aborted);
        {
            let store = SqliteCheckpointStore::open(&path).unwrap();
            store
                .begin(
                    first.clone(),
                    vec![CheckpointFileEntry {
                        path: PathBuf::from("file"),
                        content: Some("one".into()),
                    }],
                )
                .await
                .unwrap();
            store.begin(aborted.clone(), vec![]).await.unwrap();
            store
                .finalize(first.checkpoint_id, CheckpointFinalizeState::Finalized)
                .await
                .unwrap();
            store
                .finalize(aborted.checkpoint_id, CheckpointFinalizeState::Finalized)
                .await
                .unwrap();
        }

        let store = SqliteCheckpointStore::open(&path).unwrap();
        let loaded = store.load("session", 1).await.unwrap().unwrap();
        assert_eq!(loaded.0.finalize_state, CheckpointFinalizeState::Finalized);
        assert_eq!(loaded.1[0].content.as_deref(), Some("one"));
        assert_eq!(
            store
                .load("session", 2)
                .await
                .unwrap()
                .unwrap()
                .0
                .finalize_state,
            CheckpointFinalizeState::Aborted
        );
        store.truncate_after("session", 1).await.unwrap();
        assert!(store.load("session", 1).await.unwrap().is_some());
        assert!(store.load("session", 2).await.unwrap().is_none());
    }
}
