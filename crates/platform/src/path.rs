//! Platform-agnostic path representation.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A logical host path that also preserves the platform-native representation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostPath {
    /// Normalised logical path (forward slashes).
    logical: String,
    /// OS-native path.
    native: PathBuf,
}

impl HostPath {
    pub fn new(native: PathBuf) -> Self {
        let logical = native.to_string_lossy().replace('\\', "/");
        Self { logical, native }
    }

    pub fn logical(&self) -> &str {
        &self.logical
    }

    pub fn native(&self) -> &Path {
        &self.native
    }
}

impl std::fmt::Display for HostPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.logical.fmt(f)
    }
}

impl From<PathBuf> for HostPath {
    fn from(p: PathBuf) -> Self {
        Self::new(p)
    }
}
