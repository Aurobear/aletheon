//! Durable channel database backed by SQLite.
//!
//! Stores channel inbox/outbox messages, cursors, and bindings so that
//! channel offsets and messages survive daemon restarts without affecting
//! the existing objective database (`objectives.db`).

use anyhow::{Context, Result};
use fabric::channel::InboundMessage;
use rusqlite::Connection;
use rusqlite::OptionalExtension;

/// SQLite-backed channel store.
///
/// Uses a dedicated `channels.db` under `DaemonConfig.data_dir`.
/// Ownership mirrors `ObjectiveStore`: the struct holds `rusqlite::Connection`.
pub struct ChannelStore {
    pub(crate) db: Connection,
}

/// Outcome of inserting a provider message into the inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

impl ChannelStore {
    /// Open (or create + migrate) the channel store at `path`.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening channel store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::migrate(&db)?;
        Ok(Self { db })
    }

    /// Run schema migrations idempotently.
    ///
    /// Uses `PRAGMA user_version` to track the current schema version.
    /// Migration 1 creates the initial tables.
    fn migrate(db: &Connection) -> Result<()> {
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;

        if version < 1 {
            let tx = db
                .unchecked_transaction()
                .context("beginning migration transaction")?;

            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_inbox (
                    channel_id      TEXT NOT NULL,
                    message_id      TEXT NOT NULL,
                    conversation_id TEXT NOT NULL,
                    sender_id       TEXT NOT NULL,
                    payload_json    TEXT NOT NULL,
                    correlation_id  TEXT NOT NULL,
                    status          TEXT NOT NULL DEFAULT 'pending'
                                    CHECK(status IN ('pending','processing','completed','rejected','failed')),
                    result_json     TEXT,
                    attempt_count   INTEGER NOT NULL DEFAULT 0,
                    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY(channel_id, message_id)
                );

                CREATE TABLE IF NOT EXISTS channel_outbox (
                    outbox_id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id       TEXT NOT NULL,
                    conversation_id  TEXT NOT NULL,
                    payload_json     TEXT NOT NULL,
                    correlation_id   TEXT NOT NULL UNIQUE,
                    status           TEXT NOT NULL DEFAULT 'pending'
                                     CHECK(status IN ('pending','sending','sent','failed')),
                    attempt_count    INTEGER NOT NULL DEFAULT 0,
                    provider_message_id TEXT,
                    last_error       TEXT,
                    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS channel_cursor (
                    channel_id  TEXT PRIMARY KEY,
                    cursor      TEXT NOT NULL,
                    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS channel_binding (
                    channel_id  TEXT NOT NULL,
                    external_id TEXT NOT NULL,
                    principal_id TEXT NOT NULL,
                    status      TEXT NOT NULL DEFAULT 'active'
                                CHECK(status IN ('pending','active','revoked')),
                    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY(channel_id, external_id)
                );
                PRAGMA user_version = 1;",
            )
            .context("creating channel schema")?;

            tx.commit().context("committing migration transaction")?;
        }

        Ok(())
    }

    /// Returns the current `PRAGMA user_version`.
    pub fn user_version(&self) -> Result<i64> {
        self.db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(Into::into)
    }

    /// Returns true if the given table name exists in `sqlite_master`.
    pub fn table_exists(&self, name: &str) -> Result<bool> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![name],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Bind an external identity to a principal, idempotent via INSERT OR IGNORE.
    pub fn bind(&self, channel: &str, external: &str, principal: &str, status: &str) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO channel_binding (channel_id, external_id, principal_id, status)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![channel, external, principal, status],
        )?;
        Ok(())
    }

    /// Resolve the principal for a channel + external identity, only when status is 'active'.
    pub fn resolve_principal(&self, channel: &str, external: &str) -> Result<Option<String>> {
        let mut stmt = self.db.prepare(
            "SELECT principal_id FROM channel_binding
             WHERE channel_id = ?1 AND external_id = ?2 AND status = 'active'",
        )?;
        let principal: Option<String> = stmt
            .query_row(rusqlite::params![channel, external], |r| r.get(0))
            .optional()?;
        Ok(principal)
    }

    /// Insert an inbound message. Returns `Inserted` on first insert,
    /// `Duplicate` when the (channel_id, message_id) pair already exists.
    pub fn insert_inbound(&mut self, message: &InboundMessage) -> Result<InsertOutcome> {
        let payload = serde_json::to_string(message).context("serializing inbound message")?;
        let affected = self.db.execute(
            "INSERT OR IGNORE INTO channel_inbox
                (channel_id, message_id, conversation_id, sender_id, payload_json, correlation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                message.channel_id.0,
                message.message_id.0,
                message.conversation_id.0,
                message.sender_id.0,
                payload,
                message.correlation_id,
            ],
        )?;
        if affected == 1 {
            Ok(InsertOutcome::Inserted)
        } else {
            Ok(InsertOutcome::Duplicate)
        }
    }

    /// Load an inbound message by channel and message id. Returns `None` if not found.
    pub fn load_inbound(&self, channel: &str, message_id: &str) -> Result<Option<InboundMessage>> {
        let mut stmt = self.db.prepare(
            "SELECT payload_json FROM channel_inbox
             WHERE channel_id = ?1 AND message_id = ?2",
        )?;
        let payload: Option<String> = stmt
            .query_row(rusqlite::params![channel, message_id], |r| r.get(0))
            .optional()?;
        match payload {
            Some(p) => {
                let msg: InboundMessage =
                    serde_json::from_str(&p).context("deserializing inbound message")?;
                Ok(Some(msg))
            }
            None => Ok(None),
        }
    }

    /// Return pending inbound messages for a channel, up to `limit`.
    pub fn pending_inbound(&self, channel: &str, limit: usize) -> Result<Vec<InboundMessage>> {
        let mut stmt = self.db.prepare(
            "SELECT payload_json FROM channel_inbox
             WHERE channel_id = ?1 AND status = 'pending'
             ORDER BY created_at ASC
             LIMIT ?2",
        )?;
        let rows: Vec<String> = stmt
            .query_map(
                rusqlite::params![channel, limit as i64],
                |r| r.get(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut msgs = Vec::with_capacity(rows.len());
        for row in rows {
            let msg: InboundMessage =
                serde_json::from_str(&row).context("deserializing pending inbound message")?;
            msgs.push(msg);
        }
        Ok(msgs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::channel::{ChannelId, ConversationId, ExternalSenderId, MessageContent, MessageId};

    fn test_store() -> (ChannelStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels.db");
        let store = ChannelStore::open(&path).unwrap();
        (store, dir)
    }

    fn sample_inbound(message_id: &str, text: &str) -> InboundMessage {
        InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId(message_id.into()),
            conversation_id: ConversationId("1001".into()),
            sender_id: ExternalSenderId("7".into()),
            content: MessageContent::Text {
                text: text.into(),
            },
            timestamp_ms: 1_720_000_000_000,
            reply_to_action: None,
            correlation_id: "corr-1".into(),
        }
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels.db");
        ChannelStore::open(&path).unwrap();
        let store = ChannelStore::open(&path).unwrap();
        assert_eq!(store.user_version().unwrap(), 1);
        for table in [
            "channel_inbox",
            "channel_outbox",
            "channel_cursor",
            "channel_binding",
        ] {
            assert!(store.table_exists(table).unwrap(), "missing {table}");
        }
    }

    #[test]
    fn binding_resolves_only_active_principal() {
        let (store, _dir) = test_store();
        store.bind("telegram", "7", "owner", "active").unwrap();
        assert_eq!(
            store.resolve_principal("telegram", "7").unwrap().as_deref(),
            Some("owner")
        );
        assert_eq!(store.resolve_principal("telegram", "8").unwrap(), None);
    }

    #[test]
    fn rebinding_same_external_identity_is_idempotent() {
        let (store, _dir) = test_store();
        store.bind("telegram", "7", "owner", "active").unwrap();
        store.bind("telegram", "7", "owner", "active").unwrap();
        assert_eq!(
            store.resolve_principal("telegram", "7").unwrap().as_deref(),
            Some("owner")
        );
    }

    #[test]
    fn duplicate_provider_message_is_not_inserted_twice() {
        let (mut store, _dir) = test_store();
        let first = sample_inbound("42", "first");
        let second = sample_inbound("42", "changed");
        assert_eq!(
            store.insert_inbound(&first).unwrap(),
            InsertOutcome::Inserted
        );
        assert_eq!(
            store.insert_inbound(&second).unwrap(),
            InsertOutcome::Duplicate
        );
        assert_eq!(
            store
                .load_inbound("telegram", "42")
                .unwrap()
                .unwrap()
                .content,
            first.content
        );
    }
}
