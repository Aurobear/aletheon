//! Private mutation surface for the materialized Session read model.
//!
//! Implementing this trait does not grant application Session authority. Only
//! `EventSourcedSessionStore` implements Fabric's `SessionAppendStore`; it calls
//! this surface after the corresponding EventSpine fact is durable.

use anyhow::Result;
use async_trait::async_trait;
use fabric::{
    AppendOutcome, ItemId, ItemRecord, PrincipalId, SessionId, SessionReadStore, SessionRecord,
};

#[async_trait]
pub trait SessionProjectionStore: SessionReadStore {
    async fn create(&self, session: SessionRecord) -> Result<()>;

    /// Read the durable per-session sequence head (the next sequence that will
    /// be assigned). Used for O(1) append admission instead of scanning the
    /// full session history.
    async fn next_sequence(&self, session: &SessionId) -> Result<Option<u64>>;

    /// O(1) idempotency lookup by stable item id. Returns `None` when no item
    /// with that id has been committed; otherwise the persisted item for
    /// equality comparison. This replaces the full-history `load_items` scan
    /// used for append admission (S1-AUDIT-001).
    async fn item_by_id(&self, session: &SessionId, id: &ItemId) -> Result<Option<ItemRecord>>;

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome>;

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()>;

    async fn bind_principal(&self, session: &SessionId, principal: &PrincipalId) -> Result<()>;
}
