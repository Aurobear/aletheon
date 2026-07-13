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

    /// Remove a space and its bindings/overlay. Idempotent; returns whether an
    /// entry existed. Called when an ephemeral (per-turn) space is done, to
    /// prevent unbounded growth of the space table.
    pub fn release(&self, space: SpaceId) -> bool {
        self.spaces
            .lock()
            .map(|mut s| s.remove(&space).is_some())
            .unwrap_or(false)
    }

    /// Insert or update a binding. Singleton-per-space variants (Session,
    /// Agora, MemoryView, WorldProjection) replace an existing binding of the
    /// same variant in place; Artifact bindings (multi-instance) are appended.
    /// Infallible: a poisoned mutex is a no-op.
    pub fn upsert_binding(&self, space: SpaceId, binding: ContextBinding) {
        if let Ok(mut spaces) = self.spaces.lock() {
            let entry = spaces.entry(space).or_insert_with(|| empty_space(space));
            let is_multi = matches!(binding, ContextBinding::Artifact(_, _));
            if !is_multi {
                entry
                    .bindings
                    .retain(|b| std::mem::discriminant(b) != std::mem::discriminant(&binding));
            }
            entry.bindings.push(binding);
        }
    }

    /// Number of tracked spaces (observability / leak checks).
    pub fn space_count(&self) -> usize {
        self.spaces.lock().map(|s| s.len()).unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn release_is_idempotent_and_clears_entry() {
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new();
        m.set_overlay(s, "turn_input", json!("hi")).unwrap();
        assert!(m.get_space(s).is_some());
        assert_eq!(m.space_count(), 1);

        assert!(m.release(s)); // entry existed
        assert!(m.get_space(s).is_none());
        assert_eq!(m.space_count(), 0);

        assert!(!m.release(s)); // idempotent: already gone
    }

    #[test]
    fn per_turn_cycle_does_not_grow() {
        let m = InMemorySpaceManager::new();
        // Simulate the daemon per-turn create -> overlay -> release cycle.
        for i in 0..1000 {
            let s = SpaceId::new();
            m.set_overlay(s, "turn_input", json!(i)).unwrap();
            assert!(m.release(s));
        }
        assert_eq!(
            m.space_count(),
            0,
            "spaces must not accumulate across turns"
        );
    }

    #[test]
    fn upsert_replaces_singletons_appends_artifacts() {
        use fabric::types::space::{AccessMode, AgoraSpaceId, AgoraVersion, ArtifactId, SessionId};
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new();

        m.upsert_binding(
            s,
            ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(1)),
        );
        m.upsert_binding(
            s,
            ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(2)),
        );
        let b = m.get_bindings(s).unwrap();
        assert_eq!(
            b.iter()
                .filter(|x| matches!(x, ContextBinding::Agora(_, _)))
                .count(),
            1
        );
        assert!(b
            .iter()
            .any(|x| matches!(x, ContextBinding::Agora(_, AgoraVersion(2)))));

        m.upsert_binding(s, ContextBinding::Session(SessionId("x".into())));
        m.upsert_binding(s, ContextBinding::Session(SessionId("x".into())));
        let b = m.get_bindings(s).unwrap();
        assert_eq!(
            b.iter()
                .filter(|x| matches!(x, ContextBinding::Session(_)))
                .count(),
            1
        );

        m.upsert_binding(
            s,
            ContextBinding::Artifact(ArtifactId("a".into()), AccessMode::ReadOnly),
        );
        m.upsert_binding(
            s,
            ContextBinding::Artifact(ArtifactId("b".into()), AccessMode::ReadOnly),
        );
        let b = m.get_bindings(s).unwrap();
        assert_eq!(
            b.iter()
                .filter(|x| matches!(x, ContextBinding::Artifact(_, _)))
                .count(),
            2
        );
    }

    #[test]
    fn reused_space_does_not_grow_across_turns() {
        use fabric::types::space::{AgoraSpaceId, AgoraVersion, SessionId};
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new(); // one long-lived space
        for v in 0..1000u64 {
            m.upsert_binding(s, ContextBinding::Session(SessionId("sess".into())));
            m.upsert_binding(
                s,
                ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(v)),
            );
            m.set_overlay(s, "turn_input", serde_json::json!(v))
                .unwrap();
        }
        assert_eq!(m.space_count(), 1);
        assert_eq!(
            m.get_bindings(s).unwrap().len(),
            2,
            "one Session + one Agora, no accumulation"
        );
    }
}
