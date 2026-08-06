//! Private mutation surface for the materialized Session read model.
//!
//! Implementing this trait does not grant application Session authority. Only
//! `EventSourcedSessionStore` implements Fabric's `SessionAppendStore`; it calls
//! this surface after the corresponding EventSpine fact is durable.

use anyhow::Result;
use async_trait::async_trait;
use fabric::{AppendOutcome, ItemRecord, PrincipalId, SessionId, SessionReadStore, SessionRecord};

#[async_trait]
pub trait SessionProjectionStore: SessionReadStore {
    async fn create(&self, session: SessionRecord) -> Result<()>;

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
