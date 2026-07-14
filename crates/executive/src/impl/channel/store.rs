//! Durable channel database backed by SQLite.
//!
//! Stores channel inbox/outbox messages, cursors, and bindings so that
//! channel offsets and messages survive daemon restarts without affecting
//! the existing objective database (`objectives.db`).

use anyhow::{Context, Result};
use rusqlite::Connection;

/// SQLite-backed channel store.
///
/// Uses a dedicated `channels.db` under `DaemonConfig.data_dir`.
/// Ownership mirrors `ObjectiveStore`: the struct holds `rusqlite::Connection`.
pub struct ChannelStore {
    pub(crate) db: Connection,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
