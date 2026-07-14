//! Durable Google event outbox claiming and idempotent delivery.

use super::{GoogleSyncStore, SyncStoreError};
use async_trait::async_trait;
use fabric::{ExternalEventEnvelope, ExternalEventId};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait GoogleEventSink: Send + Sync {
    /// Implementations must treat `idempotency_key` as a durable unique key.
    async fn deliver(
        &self,
        idempotency_key: ExternalEventId,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub claimed: usize,
    pub delivered: usize,
    pub failed: usize,
}

#[derive(Clone)]
pub struct GoogleEventDispatcher {
    store: Arc<Mutex<GoogleSyncStore>>,
    sink: Arc<dyn GoogleEventSink>,
    owner: String,
    claim_duration_ms: i64,
}

impl GoogleEventDispatcher {
    pub fn new(
        store: Arc<Mutex<GoogleSyncStore>>,
        sink: Arc<dyn GoogleEventSink>,
        owner: String,
        claim_duration_ms: i64,
    ) -> Result<Self, SyncStoreError> {
        if owner.is_empty() || owner.len() > 256 || !(1_000..=300_000).contains(&claim_duration_ms)
        {
            return Err(SyncStoreError::InvalidInput);
        }
        Ok(Self {
            store,
            sink,
            owner,
            claim_duration_ms,
        })
    }

    pub async fn dispatch_due(
        &self,
        now_ms: i64,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<DispatchOutcome, SyncStoreError> {
        let claims = self.store.lock().unwrap().claim_outbox(
            &self.owner,
            now_ms,
            self.claim_duration_ms,
            limit,
        )?;
        let mut outcome = DispatchOutcome {
            claimed: claims.len(),
            delivered: 0,
            failed: 0,
        };
        for claim in claims {
            if cancel.is_cancelled() {
                break;
            }
            match self
                .sink
                .deliver(claim.event.id, &claim.event, cancel)
                .await
            {
                Ok(()) => {
                    if self.store.lock().unwrap().acknowledge_outbox(
                        &claim.outbox_id,
                        &self.owner,
                        now_ms,
                    )? {
                        outcome.delivered += 1;
                    }
                }
                Err(code) => {
                    let code = bounded_error_code(&code);
                    self.store.lock().unwrap().fail_outbox(
                        &claim.outbox_id,
                        &self.owner,
                        &code,
                        now_ms,
                    )?;
                    outcome.failed += 1;
                }
            }
        }
        Ok(outcome)
    }
}

fn bounded_error_code(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "delivery_failed".into();
    }
    value.chars().take(256).collect()
}
