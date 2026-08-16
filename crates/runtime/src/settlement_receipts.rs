//! Runtime-owned durable settlement receipt contract.
//!
//! Settlement idempotency is part of the AgentSupervisor lifecycle contract,
//! while the concrete database remains an adapter concern. Keeping this port
//! in Runtime prevents the Executive application layer from owning a second
//! receipt authority or opening SQLite directly.

use ::contracts::{AgentControlError, SettlementReceipt};
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::Mutex;

/// Durable, idempotent settlement receipt store owned by Runtime semantics.
#[async_trait]
pub trait SettlementReceiptStore: Send + Sync {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError>;

    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError>;
}

/// Deterministic receipt store for tests and explicitly non-durable fixtures.
#[derive(Debug, Default)]
pub struct InMemorySettlementReceiptStore {
    receipts: Mutex<HashMap<String, SettlementReceipt>>,
}

#[async_trait]
impl SettlementReceiptStore for InMemorySettlementReceiptStore {
    async fn get(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError> {
        Ok(self.receipts.lock().await.get(idempotency_key).cloned())
    }

    async fn put_if_absent(
        &self,
        receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError> {
        let mut receipts = self.receipts.lock().await;
        Ok(receipts
            .entry(receipt.idempotency_key.clone())
            .or_insert(receipt)
            .clone())
    }
}
