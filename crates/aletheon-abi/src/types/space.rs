//! ContextSpace types — versioned agora spaces, overlays, and context bindings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Key-value overlay applied on top of a space snapshot.
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

/// A context region bound into a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextBinding {
    /// Bind a session-scoped context by session identifier.
    Session(String),
    /// Bind an Agora space at a specific version.
    Agora(AgoraSpaceId, AgoraVersion),
    /// Bind a named memory view.
    MemoryView(String),
    /// Bind an artifact with an access mode.
    Artifact(String, AccessMode),
    /// Bind a world projection snapshot.
    WorldProjection(String, u64),
}
