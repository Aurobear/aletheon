//! In-memory SpaceManager implementation.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use fabric::include::space::SpaceManager;
use fabric::types::operation::ProcessId;
use fabric::types::process::{NamespaceId, SpaceId};
use fabric::types::space::{ContextBinding, ContextSpace, SpaceSnapshotId, VersionedOverlay};

/// In-memory SpaceManager that stores context-space records behind a Mutex.
pub struct InMemorySpaceManager {
    spaces: Mutex<HashMap<SpaceId, ContextSpace>>,
}

impl InMemorySpaceManager {
    /// Create a new empty space manager.
    pub fn new() -> Self {
        Self {
            spaces: Mutex::new(HashMap::new()),
        }
    }

    /// Return a clone of a stored space (for tests and TUI snapshots).
    pub fn get_space(&self, space: SpaceId) -> Option<ContextSpace> {
        let spaces = self.spaces.lock().ok()?;
        spaces.get(&space).cloned()
    }

    /// Return a clone of the bindings stored for a space (for testing).
    pub fn get_bindings(&self, space: SpaceId) -> Option<Vec<ContextBinding>> {
        self.get_space(space).map(|s| s.bindings)
    }

    /// Set a private overlay key without touching parent/shared bindings.
    pub fn set_overlay(
        &self,
        space: SpaceId,
        key: impl Into<String>,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut spaces = self
            .spaces
            .lock()
            .map_err(|e| anyhow::anyhow!("space mutex poisoned: {}", e))?;
        let entry = spaces.entry(space).or_insert_with(|| empty_space(space));
        entry.overlay.entries.insert(key.into(), value);
        Ok(())
    }
}

impl Default for InMemorySpaceManager {
    fn default() -> Self {
        Self::new()
    }
}

fn empty_space(id: SpaceId) -> ContextSpace {
    ContextSpace {
        id,
        owner: ProcessId::new(),
        parent_snapshot: None,
        bindings: Vec::new(),
        overlay: VersionedOverlay::default(),
        namespace: NamespaceId("default".into()),
    }
}

#[async_trait]
impl SpaceManager for InMemorySpaceManager {
    async fn fork_space(&self, parent: SpaceId, owner: ProcessId) -> anyhow::Result<SpaceId> {
        let child_id = SpaceId::new();
        let mut spaces = self
            .spaces
            .lock()
            .map_err(|e| anyhow::anyhow!("space mutex poisoned: {}", e))?;
        let parent_space = spaces
            .entry(parent)
            .or_insert_with(|| empty_space(parent))
            .clone();
        let child = ContextSpace {
            id: child_id,
            owner,
            parent_snapshot: Some(SpaceSnapshotId::new()),
            bindings: parent_space
                .bindings
                .iter()
                .map(ContextBinding::fork_inherited)
                .collect(),
            overlay: VersionedOverlay::default(),
            namespace: parent_space.namespace,
        };
        spaces.insert(child_id, child);
        Ok(child_id)
    }

    async fn attach_region(&self, space: SpaceId, binding: ContextBinding) -> anyhow::Result<()> {
        let mut spaces = self
            .spaces
            .lock()
            .map_err(|e| anyhow::anyhow!("space mutex poisoned: {}", e))?;
        let entry = spaces.entry(space).or_insert_with(|| empty_space(space));
        entry.bindings.push(binding);
        Ok(())
    }
}
