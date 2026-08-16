//! SQLite adapter for the physical channel projection store.
//!
//! Gateway owns the neutral `ChannelProjectionStore` contract. The concrete
//! SQLite implementation remains in `adapters-sqlite`; this composition
//! adapter is the only crate that binds the two types together, keeping both
//! the Gateway and persistence crates free of an inward dependency.

use crate::{ChannelStore, InsertOutcome};
use gateway::channel::{InboundMessage, OutboundMessage};
use gateway::ports::{ChannelInsertOutcome, ChannelProjectionStore};

/// Concrete composition wrapper. Keeping this local to the persistence edge satisfies
/// Rust's orphan rules without making the persistence crate depend inward on
/// Gateway; composition roots pass this wrapper to `ChannelRouter`.
pub struct SqliteChannelProjectionStore {
    inner: ChannelStore,
}

impl SqliteChannelProjectionStore {
    pub fn new(inner: ChannelStore) -> Self {
        Self { inner }
    }
}

impl ChannelProjectionStore for SqliteChannelProjectionStore {
    fn insert_inbound(&mut self, message: &InboundMessage) -> anyhow::Result<ChannelInsertOutcome> {
        Ok(
            match ChannelStore::insert_inbound(&mut self.inner, message)? {
                InsertOutcome::Inserted => ChannelInsertOutcome::Inserted,
                InsertOutcome::Duplicate => ChannelInsertOutcome::Duplicate,
            },
        )
    }

    fn resolve_principal(&self, channel: &str, external: &str) -> anyhow::Result<Option<String>> {
        Ok(ChannelStore::resolve_principal(
            &self.inner,
            channel,
            external,
        )?)
    }

    fn complete_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        next_cursor: &str,
        outbound: &OutboundMessage,
    ) -> anyhow::Result<()> {
        Ok(ChannelStore::complete_inbound(
            &mut self.inner,
            channel,
            message_id,
            next_cursor,
            outbound,
        )?)
    }

    fn reject_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        next_cursor: &str,
    ) -> anyhow::Result<()> {
        Ok(ChannelStore::reject_inbound(
            &mut self.inner,
            channel,
            message_id,
            next_cursor,
        )?)
    }

    fn fail_inbound(&self, channel: &str, message_id: &str, error: &str) -> anyhow::Result<()> {
        Ok(ChannelStore::fail_inbound(
            &self.inner,
            channel,
            message_id,
            error,
        )?)
    }

    fn enqueue_outbound(&self, channel: &str, outbound: &OutboundMessage) -> anyhow::Result<bool> {
        Ok(ChannelStore::enqueue_outbound(
            &self.inner,
            channel,
            outbound,
        )?)
    }

    fn mark_outbound_sent(&self, correlation_id: &str, provider_id: &str) -> anyhow::Result<()> {
        Ok(ChannelStore::mark_outbound_sent(
            &self.inner,
            correlation_id,
            provider_id,
        )?)
    }

    fn mark_outbound_failed(&self, correlation_id: &str, error: &str) -> anyhow::Result<()> {
        Ok(ChannelStore::mark_outbound_failed(
            &self.inner,
            correlation_id,
            error,
        )?)
    }

    fn pending_inbound(&self, channel: &str, limit: usize) -> anyhow::Result<Vec<InboundMessage>> {
        Ok(ChannelStore::pending_inbound(&self.inner, channel, limit)?)
    }

    fn pending_outbox(&self, channel: &str, limit: usize) -> anyhow::Result<Vec<OutboundMessage>> {
        Ok(ChannelStore::pending_outbox(&self.inner, channel, limit)?)
    }
}
