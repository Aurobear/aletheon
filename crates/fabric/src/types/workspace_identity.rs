//! Canonical host-derived workspace identity shared by workspace subsystems.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Canonical workspace identity, resisting path-alias and symlink bypass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    /// Normalized git remote fingerprint when available, else `None`.
    pub repo_fingerprint: Option<String>,
}

impl WorkspaceIdentity {
    /// Fail-closed identity comparison used by checkpoint restoration and trust.
    pub fn matches(&self, other: &WorkspaceIdentity) -> bool {
        self == other
    }
}
