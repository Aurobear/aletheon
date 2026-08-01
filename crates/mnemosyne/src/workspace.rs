//! Host-derived workspace memory identity.

use fabric::WorkspaceIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Opaque, stable workspace key used by memory scopes and supplemental bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceMemoryKey(String);

impl WorkspaceMemoryKey {
    pub const MAX_IDENTITY_BYTES: usize = 512;

    /// Derive a key only from host-verified workspace identity material.
    pub fn derive(
        workspace: &WorkspaceIdentity,
        machine_installation_id: &str,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            workspace.canonical_path.is_absolute(),
            "canonical workspace path must be absolute"
        );
        if let Some(repo_fingerprint) = &workspace.repo_fingerprint {
            anyhow::ensure!(
                !repo_fingerprint.trim().is_empty(),
                "repository fingerprint is required when present"
            );
            anyhow::ensure!(
                repo_fingerprint.len() <= Self::MAX_IDENTITY_BYTES,
                "repository fingerprint exceeds byte limit"
            );
            return Ok(Self(format!("ws:repo:{repo_fingerprint}")));
        }

        anyhow::ensure!(
            !machine_installation_id.trim().is_empty(),
            "machine installation ID is required"
        );
        anyhow::ensure!(
            machine_installation_id.len() <= Self::MAX_IDENTITY_BYTES,
            "machine installation ID exceeds byte limit"
        );

        let canonical_path = workspace.canonical_path.as_os_str().as_encoded_bytes();
        let mut hasher = Sha256::new();
        hasher.update(b"aletheon.workspace-memory-key.v1\0");
        update_bounded_component(&mut hasher, machine_installation_id.as_bytes());
        update_bounded_component(&mut hasher, canonical_path);
        Ok(Self(format!("ws:local:{:x}", hasher.finalize())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Rehydrate a key already derived and verified by the host or durable store.
    pub fn from_verified(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        anyhow::ensure!(
            value.starts_with("ws:repo:") || value.starts_with("ws:local:"),
            "workspace memory key has an unsupported prefix"
        );
        anyhow::ensure!(
            value.len() > "ws:repo:".len() && value.len() <= Self::MAX_IDENTITY_BYTES + 16,
            "workspace memory key is empty or exceeds byte limit"
        );
        Ok(Self(value))
    }
}

fn update_bounded_component(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

impl AsRef<str> for WorkspaceMemoryKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for WorkspaceMemoryKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
