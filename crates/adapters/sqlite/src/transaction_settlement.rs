//! Durable Host transaction settlement receipts.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};

use application::settlement::{HostSettlementReceipt, TransactionSettlementStore};

pub struct SqliteTransactionSettlementStore {
    connection: Mutex<Connection>,
}

impl SqliteTransactionSettlementStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path).context("open transaction settlement sqlite")?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS transaction_settlements (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               settlement_id TEXT NOT NULL UNIQUE,
               session_id TEXT NOT NULL,
               transaction_id TEXT NOT NULL,
               receipt_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS transaction_settlement_lookup
               ON transaction_settlements(session_id, transaction_id, sequence);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[async_trait]
impl TransactionSettlementStore for SqliteTransactionSettlementStore {
    async fn append(&self, receipt: &HostSettlementReceipt) -> anyhow::Result<()> {
        let encoded = serde_json::to_string(receipt)?;
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing = connection
            .query_row(
                "SELECT receipt_json FROM transaction_settlements WHERE settlement_id = ?1",
                params![receipt.settlement_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            anyhow::ensure!(existing == encoded, "settlement idempotency conflict");
            return Ok(());
        }
        connection.execute(
            "INSERT INTO transaction_settlements
             (settlement_id, session_id, transaction_id, receipt_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                receipt.settlement_id,
                receipt.session_id,
                receipt.transaction_id,
                encoded
            ],
        )?;
        Ok(())
    }

    async fn latest(
        &self,
        session_id: &str,
        transaction_id: &str,
    ) -> anyhow::Result<Option<HostSettlementReceipt>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let encoded = connection
            .query_row(
                "SELECT receipt_json FROM transaction_settlements
                 WHERE session_id = ?1 AND transaction_id = ?2
                 ORDER BY sequence DESC LIMIT 1",
                params![session_id, transaction_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        encoded
            .map(|encoded| serde_json::from_str(&encoded).map_err(anyhow::Error::from))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> HostSettlementReceipt {
        HostSettlementReceipt {
            settlement_id: "settlement-1".into(),
            transaction_id: "transaction-1".into(),
            session_id: "session-1".into(),
            workspace_version: "version-1".into(),
            decision: ::contracts::TransactionSettlementDecision::Accepted,
            finding_ids: vec![],
            validation_receipt_refs: vec!["artifact://validation".into()],
            validation_omissions: vec![],
            reason: "accepted".into(),
        }
    }

    #[tokio::test]
    async fn settlement_receipt_is_durable_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settlements.sqlite");
        let store = SqliteTransactionSettlementStore::open(&path).unwrap();
        store.append(&receipt()).await.unwrap();
        store.append(&receipt()).await.unwrap();
        drop(store);

        let reopened = SqliteTransactionSettlementStore::open(path).unwrap();
        assert_eq!(
            reopened.latest("session-1", "transaction-1").await.unwrap(),
            Some(receipt())
        );
    }
}
