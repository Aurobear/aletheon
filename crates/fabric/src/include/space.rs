//! Space manager contract — fork spaces and attach context regions.

use crate::types::operation::ProcessId;
use crate::types::process::SpaceId;
use crate::types::space::ContextBinding;
use async_trait::async_trait;

/// Manages ContextSpace instances: fork new spaces from a parent and attach
/// context regions (bindings) to an existing space.
#[async_trait]
pub trait SpaceManager: Send + Sync {
    /// Fork a new child space from a parent space, owned by the given process.
    async fn fork_space(&self, parent: SpaceId, owner: ProcessId) -> anyhow::Result<SpaceId>;

    /// Attach a context binding to a space.
    async fn attach_region(&self, space: SpaceId, binding: ContextBinding) -> anyhow::Result<()>;
}
