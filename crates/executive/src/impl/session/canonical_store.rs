//! Canonical transactional Session/Turn/Item history store.

use std::{path::Path, sync::Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use fabric::{
    AppendOutcome, ContentBlock, ItemId, ItemPayload, ItemRecord, Message, Role,
    SessionAppendStore, SessionId, SessionRecord, SESSION_SCHEMA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub struct CanonicalSessionStore {
    connection: Mutex<Connection>,
}

pub fn default_session_db_path() -> std::path::PathBuf {
    fabric::paths::xdg_data_dir().join("sessions-v1.db")
}

/// Canonical session database for an explicitly owned user state root.
pub fn session_db_path(state_root: &Path) -> std::path::PathBuf {
    state_root.join("sessions-v1.db")
}

impl CanonicalSessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS sessions(
               session_id TEXT PRIMARY KEY,
               schema_version INTEGER NOT NULL,
               record_json TEXT NOT NULL,
               next_sequence INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_items(
               session_id TEXT NOT NULL,
               sequence INTEGER NOT NULL,
               item_id TEXT NOT NULL UNIQUE,
               turn_id TEXT NOT NULL,
               item_json TEXT NOT NULL,
               PRIMARY KEY(session_id, sequence),
               FOREIGN KEY(session_id) REFERENCES sessions(session_id)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn validate_session(session: &SessionRecord) -> Result<()> {
        if session.schema_version != SESSION_SCHEMA_VERSION {
            bail!(
                "unsupported session schema version {}",
                session.schema_version
            );
        }
        Ok(())
    }

    fn validate_item(session: &SessionId, expected: u64, item: &ItemRecord) -> Result<()> {
        if item.schema_version != SESSION_SCHEMA_VERSION {
            bail!("unsupported item schema version {}", item.schema_version);
        }
        if &item.session_id != session {
            bail!("item session does not match append target");
        }
        if item.sequence != expected {
            bail!(
                "item sequence {} does not match expected {}",
                item.sequence,
                expected
            );
        }
        Ok(())
    }
}

#[async_trait]
impl SessionAppendStore for CanonicalSessionStore {
    async fn create(&self, session: SessionRecord) -> Result<()> {
        Self::validate_session(&session)?;
        let json = serde_json::to_string(&session)?;
        let connection = self.connection.lock().unwrap();
        let existing = connection
            .query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session.id.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != json {
                bail!("session creation conflicts with persisted content");
            }
            return Ok(());
        }
        connection
            .execute(
                "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,1)",
                params![session.id.0, session.schema_version, json],
            )
            .context("create canonical session")?;
        Ok(())
    }

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome> {
        Self::validate_item(session, expected_sequence, &item)?;
        let item_json = serde_json::to_string(&item)?;
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = tx
            .query_row(
                "SELECT item_json FROM session_items WHERE item_id=?1",
                params![item.id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing != item_json {
                bail!("item id retry conflicts with persisted content");
            }
            tx.commit()?;
            return Ok(AppendOutcome::AlreadyPresent);
        }
        let next: u64 = tx
            .query_row(
                "SELECT next_sequence FROM sessions WHERE session_id=?1",
                params![session.0],
                |row| row.get(0),
            )
            .context("canonical session not found")?;
        if next != expected_sequence {
            bail!("sequence conflict: expected {expected_sequence}, current {next}");
        }
        tx.execute(
            "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) VALUES(?1,?2,?3,?4,?5)",
            params![session.0, item.sequence, item.id.0.to_string(), item.turn_id.0.to_string(), item_json],
        )?;
        tx.execute(
            "UPDATE sessions SET next_sequence=?2 WHERE session_id=?1",
            params![session.0, next + 1],
        )?;
        tx.commit()?;
        Ok(AppendOutcome::Appended)
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()> {
        Self::validate_session(&child)?;
        let expected_parent = child
            .parent
            .as_ref()
            .context("fork child missing parent metadata")?;
        if &expected_parent.session_id != parent
            || expected_parent.through_sequence != through_sequence
        {
            bail!("fork metadata does not match request");
        }
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parent_next: u64 = tx.query_row(
            "SELECT next_sequence FROM sessions WHERE session_id=?1",
            params![parent.0],
            |r| r.get(0),
        )?;
        if through_sequence >= parent_next {
            bail!("parent sequence {through_sequence} does not exist");
        }
        tx.execute(
            "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,?4)",
            params![child.id.0, child.schema_version, serde_json::to_string(&child)?, through_sequence + 1],
        )?;
        let mut stmt = tx.prepare("SELECT item_json FROM session_items WHERE session_id=?1 AND sequence<=?2 ORDER BY sequence")?;
        let rows: Vec<String> = stmt
            .query_map(params![parent.0, through_sequence], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for json in rows {
            let mut item: ItemRecord = serde_json::from_str(&json)?;
            item.id = ItemId::new();
            item.session_id = child.id.clone();
            tx.execute(
                "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) VALUES(?1,?2,?3,?4,?5)",
                params![child.id.0, item.sequence, item.id.0.to_string(), item.turn_id.0.to_string(), serde_json::to_string(&item)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>> {
        let json = self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        json.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>> {
        let connection = self.connection.lock().unwrap();
        let mut stmt = connection.prepare(
            "SELECT item_json FROM session_items WHERE session_id=?1 AND sequence>?2 ORDER BY sequence"
        )?;
        let items = stmt
            .query_map(params![session.0, after.unwrap_or(0)], |r| {
                r.get::<_, String>(0)
            })?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect();
        items
    }
}

pub fn project_messages(items: &[ItemRecord]) -> Result<Vec<Message>> {
    let mut previous = 0;
    let mut messages = Vec::new();
    for item in items {
        if item.sequence <= previous {
            bail!(
                "items are duplicate or out of order at sequence {}",
                item.sequence
            );
        }
        previous = item.sequence;
        let message = match &item.payload {
            ItemPayload::UserMessage { content } => Some(Message::user(content)),
            ItemPayload::AssistantMessage { content } => Some(Message::assistant(content)),
            ItemPayload::SystemNotice { content } => Some(Message::system(content)),
            ItemPayload::ToolCall {
                call_id,
                name,
                input,
            } => Some(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }],
            }),
            ItemPayload::ToolResult {
                call_id,
                content,
                is_error,
                ..
            } => Some(Message::tool_result(call_id, content, *is_error)),
            ItemPayload::ContextProjection { .. } => None,
        };
        if let Some(message) = message {
            messages.push(message);
        }
    }
    Ok(messages)
}
