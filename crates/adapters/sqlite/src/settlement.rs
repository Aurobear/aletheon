//! SQLite adapter for Runtime settlement receipts.

use ::contracts::{AgentControlError, AgentControlErrorKind, SettlementReceipt};
use async_trait::async_trait;
use rusqlite::OptionalExtension;

/// Durable idempotency receipts for Runtime Agent settlement.
pub struct SqliteSettlementReceiptStore {
    connection: parking_lot::Mutex<rusqlite::Connection>,
}

impl SqliteSettlementReceiptStore {
    /// Apply the settlement receipt schema before composition opens the store.
    /// `open` is deliberately DDL-free; callers must invoke this from the
    /// migration phase.
    pub fn migrate(path: impl AsRef<std::path::Path>) -> Result<(), AgentControlError> {
        let connection = rusqlite::Connection::open(path).map_err(persistence)?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS agent_settlement_receipts (
                    idempotency_key TEXT PRIMARY KEY NOT NULL,
                    receipt_json TEXT NOT NULL
                );",
            )
            .map_err(persistence)
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, AgentControlError> {
        let connection = rusqlite::Connection::open(path).map_err(persistence)?;
        Ok(Self {
            connection: parking_lot::Mutex::new(connection),
        })
    }
}

#[async_trait]
impl runtime::SettlementReceiptStore for SqliteSettlementReceiptStore {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError> {
        let connection = self.connection.lock();
        let encoded = connection
            .query_row(
                "SELECT receipt_json FROM agent_settlement_receipts WHERE idempotency_key=?1",
                rusqlite::params![idempotency_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(persistence)?;
        encoded
            .map(|value| serde_json::from_str(&value).map_err(persistence))
            .transpose()
    }

    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError> {
        let encoded = serde_json::to_string(&receipt).map_err(persistence)?;
        let connection = self.connection.lock();
        connection
            .execute(
                "INSERT OR IGNORE INTO agent_settlement_receipts(idempotency_key,receipt_json)
                 VALUES (?1,?2)",
                rusqlite::params![&receipt.idempotency_key, encoded],
            )
            .map_err(persistence)?;
        let authoritative = connection
            .query_row(
                "SELECT receipt_json FROM agent_settlement_receipts WHERE idempotency_key=?1",
                rusqlite::params![&receipt.idempotency_key],
                |row| row.get::<_, String>(0),
            )
            .map_err(persistence)?;
        serde_json::from_str(&authoritative).map_err(persistence)
    }
}

fn persistence(error: impl std::fmt::Display) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Persistence,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteSettlementReceiptStore;
    use ::contracts::{SettlementReceipt, SettlementTerminal};
    use rusqlite::OptionalExtension;

    fn receipt(key: &str) -> SettlementReceipt {
        SettlementReceipt {
            idempotency_key: key.into(),
            agent_id: "agent-1".into(),
            attempt_id: "attempt-1".into(),
            generation: "generation-1".into(),
            terminal: SettlementTerminal::Completed,
            settled_at_ms: 1,
            released_leases: Vec::new(),
            reparented: Vec::new(),
        }
    }

    #[tokio::test]
    async fn persists_and_replays_authoritative_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settlement.db");
        SqliteSettlementReceiptStore::migrate(&path).unwrap();
        let store = SqliteSettlementReceiptStore::open(&path).unwrap();
        let first = runtime::SettlementReceiptStore::put_if_absent(&store, receipt("key-1"))
            .await
            .unwrap();
        let replay = runtime::SettlementReceiptStore::put_if_absent(&store, receipt("key-1"))
            .await
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(
            runtime::SettlementReceiptStore::get(&store, "key-1")
                .await
                .unwrap(),
            Some(first)
        );
    }

    #[test]
    fn open_does_not_create_schema_without_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unmigrated.db");
        let _store = SqliteSettlementReceiptStore::open(&path).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        let table: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_settlement_receipts'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(table, None);
    }
}
