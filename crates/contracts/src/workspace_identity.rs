//! Canonical host-derived workspace identity shared across subsystem ports.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    pub repo_fingerprint: Option<String>,
}

impl WorkspaceIdentity {
    pub fn matches(&self, other: &WorkspaceIdentity) -> bool {
        self == other
    }
}
