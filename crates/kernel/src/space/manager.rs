//! In-memory SpaceManager implementation.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use fabric::include::space::SpaceManager;
use fabric::types::operation::ProcessId;
use fabric::types::process::SpaceId;
use fabric::types::space::{ContextBinding, VersionedOverlay};

/// In-memory SpaceManager that stores space state behind a Mutex.
pub struct InMemorySpaceManager {
    spaces: Mutex<HashMap<SpaceId, (Vec<ContextBinding>, VersionedOverlay)>>,
}

impl InMemorySpaceManager {
    /// Create a new empty space manager.
    pub fn new() -> Self {
        Self {
            spaces: Mutex::new(HashMap::new()),
        }
    }

    /// Return a clone of the bindings stored for a space (for testing).
    pub fn get_bindings(&self, space: SpaceId) -> Option<Vec<ContextBinding>> {
        let spaces = self.spaces.lock().ok()?;
        spaces.get(&space).map(|(bindings, _)| bindings.clone())
    }
}

impl Default for InMemorySpaceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SpaceManager for InMemorySpaceManager {
    async fn fork_space(&self, parent: SpaceId, _owner: ProcessId) -> anyhow::Result<SpaceId> {
        let child_id = SpaceId::new();
        let mut spaces = self
            .spaces
            .lock()
            .map_err(|e| anyhow::anyhow!("space mutex poisoned: {}", e))?;
        let (parent_bindings, _parent_overlay) = spaces.get(&parent).cloned().unwrap_or_default();
        spaces.insert(child_id, (parent_bindings, VersionedOverlay::default()));
        Ok(child_id)
    }

    async fn attach_region(&self, space: SpaceId, binding: ContextBinding) -> anyhow::Result<()> {
        let mut spaces = self
            .spaces
            .lock()
            .map_err(|e| anyhow::anyhow!("space mutex poisoned: {}", e))?;
        let entry = spaces.entry(space).or_default();
        entry.0.push(binding);
        Ok(())
    }
}
