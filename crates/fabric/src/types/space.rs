//! ContextSpace types — versioned agora spaces, overlays, and context bindings.

use crate::types::operation::ProcessId;
use crate::types::process::{NamespaceId, SpaceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Identifier for a user/session continuity context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Identifier for an Agora (shared context space).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgoraSpaceId(pub String);

/// Monotonic version counter for an Agora space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgoraVersion(pub u64);

impl AgoraVersion {
    /// Bump to the next version.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Snapshot identifier backed by a UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpaceSnapshotId(pub uuid::Uuid);

impl SpaceSnapshotId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for SpaceSnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryViewId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldProjectionId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectionVersion(pub u64);

/// Key-value private overlay applied on top of version-pinned context bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionedOverlay {
    pub entries: HashMap<String, serde_json::Value>,
}

/// Access mode for a bound context region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

/// A process-private context space: references are version-pinned, while
/// private turn/process data lives in the overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSpace {
    pub id: SpaceId,
    pub owner: ProcessId,
    pub parent_snapshot: Option<SpaceSnapshotId>,
    pub bindings: Vec<ContextBinding>,
    pub overlay: VersionedOverlay,
    pub namespace: NamespaceId,
}

/// A context region bound into a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextBinding {
    /// Bind a session-scoped context by session identifier.
    Session(SessionId),
    /// Bind an Agora space at a specific version.
    Agora(AgoraSpaceId, AgoraVersion),
    /// Bind a named memory view.
    MemoryView(MemoryViewId),
    /// Bind an artifact with an access mode.
    Artifact(ArtifactId, AccessMode),
    /// Bind a world projection snapshot.
    WorldProjection(WorldProjectionId, ProjectionVersion),
}

impl ContextBinding {
    /// Fork semantics: children inherit visibility but not write authority.
    pub fn fork_inherited(&self) -> Self {
        match self {
            Self::Artifact(id, AccessMode::ReadWrite) => {
                Self::Artifact(id.clone(), AccessMode::ReadOnly)
            }
            other => other.clone(),
        }
    }
}

impl From<&str> for SessionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for SessionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for MemoryViewId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for MemoryViewId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ArtifactId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for ArtifactId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for WorldProjectionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for WorldProjectionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}
