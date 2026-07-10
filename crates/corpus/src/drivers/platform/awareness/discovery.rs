//! Agent discovery via Unix socket scan.
//!
//! Scans `/var/run/aletheon/*.sock` to find running agents.
//! Future phases will add mDNS (L3) and WAN (L4) discovery.

use super::{AgentId, AgentInfo, Endpoint};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Default socket directory for aletheon.
pub const DEFAULT_SOCKET_DIR: &str = fabric::paths::SOCKET_DIR;

/// Discovers agents by scanning a Unix socket directory.
pub struct AgentDiscovery {
    /// Directory to scan for .sock files.
    socket_dir: PathBuf,
}

impl AgentDiscovery {
    /// Create a discovery scanner for the default socket directory.
    pub fn new() -> Self {
        Self {
            socket_dir: PathBuf::from(DEFAULT_SOCKET_DIR),
        }
    }

    /// Create a discovery scanner for a custom directory (useful for testing).
    pub fn with_dir(socket_dir: impl Into<PathBuf>) -> Self {
        Self {
            socket_dir: socket_dir.into(),
        }
    }

    /// Scan the socket directory and return discovered agents.
    ///
    /// Each `.sock` file is treated as a potential agent endpoint.
    /// The agent ID is derived from the socket filename stem (expected to be a UUID).
    pub async fn scan(&self) -> Result<Vec<AgentInfo>> {
        let dir = &self.socket_dir;

        if !dir.exists() {
            info!(
                "Socket directory {} does not exist, no agents discovered",
                dir.display()
            );
            return Ok(Vec::new());
        }

        let mut entries = tokio::fs::read_dir(dir)
            .await
            .with_context(|| format!("Failed to read socket directory: {}", dir.display()))?;

        let mut agents = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Only process .sock files
            match path.extension().and_then(|e| e.to_str()) {
                Some("sock") => {}
                _ => continue,
            }

            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => {
                    warn!("Skipping socket with non-UTF8 filename: {}", path.display());
                    continue;
                }
            };

            let id = match AgentId::parse(stem) {
                Ok(id) => id,
                Err(_) => {
                    debug!("Skipping socket with non-UUID name: {}", path.display());
                    continue;
                }
            };

            // Verify the socket is actually connectable (exists and is a socket)
            if !is_socket_usable(&path).await {
                debug!("Socket not usable, skipping: {}", path.display());
                continue;
            }

            let info = AgentInfo::new(
                id,
                super::AgentKind::Worker, // default, real kind would come from handshake
                Endpoint::UnixSocket(path),
            );
            agents.push(info);
        }

        info!("Discovered {} agents in {}", agents.len(), dir.display());
        Ok(agents)
    }

    /// Get the socket directory being scanned.
    pub fn socket_dir(&self) -> &Path {
        &self.socket_dir
    }
}

impl Default for AgentDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a path points to an existing Unix socket.
async fn is_socket_usable(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(meta) => {
            // On Unix, check file type for socket
            use std::os::unix::fs::FileTypeExt;
            meta.file_type().is_socket()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_dir() {
        let discovery = AgentDiscovery::new();
        assert_eq!(discovery.socket_dir(), Path::new(DEFAULT_SOCKET_DIR));
    }

    #[test]
    fn test_custom_socket_dir() {
        let discovery = AgentDiscovery::with_dir("/tmp/test-sockets");
        assert_eq!(discovery.socket_dir(), Path::new("/tmp/test-sockets"));
    }

    #[tokio::test]
    async fn test_scan_nonexistent_directory() {
        let discovery = AgentDiscovery::with_dir("/tmp/aletheon-test-nonexistent");
        let agents = discovery.scan().await.unwrap();
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn test_scan_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = AgentDiscovery::with_dir(dir.path());
        let agents = discovery.scan().await.unwrap();
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn test_scan_skips_non_uuid_sockets() {
        let dir = tempfile::tempdir().unwrap();

        // Create a file that looks like a socket but has a non-UUID name
        let fake_sock = dir.path().join("not-a-uuid.sock");
        tokio::fs::write(&fake_sock, b"").await.unwrap();

        let discovery = AgentDiscovery::with_dir(dir.path());
        let agents = discovery.scan().await.unwrap();
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn test_scan_skips_non_socket_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create a regular file with a UUID name and .sock extension
        let uuid_name = format!("{}.sock", uuid::Uuid::new_v4());
        let fake_sock = dir.path().join(&uuid_name);
        tokio::fs::write(&fake_sock, b"not a real socket")
            .await
            .unwrap();

        let discovery = AgentDiscovery::with_dir(dir.path());
        let agents = discovery.scan().await.unwrap();
        // The file exists but is not a socket, so it should be skipped
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn test_scan_discovers_real_sockets() {
        let dir = tempfile::tempdir().unwrap();

        // Create actual Unix sockets
        use tokio::net::UnixListener;

        let agent_id = AgentId::new();
        let sock_path = dir.path().join(format!("{}.sock", agent_id));
        let _listener = UnixListener::bind(&sock_path).unwrap();

        let discovery = AgentDiscovery::with_dir(dir.path());
        let agents = discovery.scan().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent_id);
        assert_eq!(agents[0].endpoint, Endpoint::UnixSocket(sock_path));
    }
}
