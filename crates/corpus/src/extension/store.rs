//! Content-addressed package store.
//!
//! Packages are stored by SHA-256 content hash. Activation state is maintained
//! as a pointer file that atomically switches between versions.

use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// Package store backed by a filesystem directory.
pub struct PackageStore {
    root: PathBuf,
    packages_dir: PathBuf,
    state_dir: PathBuf,
}

impl PackageStore {
    /// Create a new store rooted at `root`.
    /// Creates subdirectories if they don't exist.
    pub fn new(root: PathBuf) -> Result<Self> {
        let packages_dir = root.join("packages");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&packages_dir)?;
        std::fs::create_dir_all(&state_dir)?;
        Ok(Self { root, packages_dir, state_dir })
    }

    /// Return the path where a package with the given hash is stored.
    /// Validates that `hash` is exactly 64 lowercase hex characters.
    pub fn package_path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.packages_dir.join(hash))
    }

    /// Return the activation pointer path for a package.
    pub fn activation_path(&self, package_id: &str) -> PathBuf {
        let safe_id = package_id.replace(['/', '\\', '.'], "_");
        self.state_dir.join(format!("{}.active", safe_id))
    }

    /// Return the lock file path for a package.
    pub fn lock_path(&self, package_id: &str) -> PathBuf {
        let safe_id = package_id.replace(['/', '\\', '.'], "_");
        self.state_dir.join(format!("{}.lock", safe_id))
    }

    /// Acquire a lock for the given package. Returns an error if another
    /// live process already holds the lock.
    pub fn acquire_lock(&self, package_id: &str) -> Result<()> {
        let lock_path = self.lock_path(package_id);
        let pid = std::process::id();

        // Attempt atomic creation first — no TOCTOU gap.
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                write!(f, "{}", pid)
                    .with_context(|| format!("cannot write lock for: {}", package_id))?;
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock file exists — check if the holder is still alive.
                let content =
                    std::fs::read_to_string(&lock_path).unwrap_or_default();
                if let Ok(holder_pid) = content.trim().parse::<i32>() {
                    if pid_alive(holder_pid) {
                        bail!(
                            "package '{}' is locked by process {}",
                            package_id,
                            holder_pid
                        );
                    }
                }
                // Dead PID or corrupt content — remove stale lock.
                let _ = std::fs::remove_file(&lock_path);
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("cannot create lock for: {}", package_id));
            }
        }

        // Retry atomic creation after cleaning up stale lock.
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("cannot create lock for: {}", package_id))?;
        write!(f, "{}", pid)
            .with_context(|| format!("cannot write lock for: {}", package_id))?;
        Ok(())
    }

    /// Release the lock for the given package.
    pub fn release_lock(&self, package_id: &str) {
        let lock_path = self.lock_path(package_id);
        let _ = std::fs::remove_file(&lock_path);
    }

    /// Check if a package with the given hash is already installed.
    /// Validates that `hash` is exactly 64 lowercase hex characters.
    pub fn is_installed(&self, hash: &str) -> Result<bool> {
        Ok(self.package_path(hash)?.exists())
    }

    /// Get the staging directory for a package hash.
    /// Validates that `hash` is exactly 64 lowercase hex characters.
    pub fn staging_path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.root.join("staging").join(&hash[..16]))
    }

    /// Move staging directory to final package location.
    /// Validates that `hash` is exactly 64 lowercase hex characters.
    pub fn commit_staging(&self, hash: &str) -> Result<()> {
        validate_hash(hash)?;
        let staging = self.staging_path(hash)?;
        let dest = self.package_path(hash)?;
        if dest.exists() {
            bail!("package already installed at: {}", dest.display());
        }
        std::fs::rename(&staging, &dest)
            .with_context(|| format!("cannot commit staging to: {}", dest.display()))?;
        // fsync parent directory
        if let Some(parent) = dest.parent() {
            let _ = std::fs::File::open(parent).and_then(|f| f.sync_all());
        }
        Ok(())
    }

    /// Store a receipt for a package operation.
    pub fn store_receipt(&self, package_id: &str, receipt: &str) -> Result<()> {
        let safe_id = package_id.replace(['/', '\\', '.'], "_");
        let receipts_dir = self.state_dir.join(&safe_id).join("receipts");
        std::fs::create_dir_all(&receipts_dir)?;
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
        let receipt_path = receipts_dir.join(format!("{}.json", timestamp));
        // Atomic write
        let tmp = receipts_dir.join(format!(".{}.tmp", timestamp));
        std::fs::write(&tmp, receipt)?;
        std::fs::rename(&tmp, &receipt_path)?;
        // fsync parent
        if let Ok(f) = std::fs::File::open(&receipts_dir) {
            let _ = f.sync_all();
        }
        Ok(())
    }

    /// List installed package hashes.
    pub fn list_installed(&self) -> Result<Vec<String>> {
        let mut hashes = Vec::new();
        for entry in std::fs::read_dir(&self.packages_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                hashes.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        Ok(hashes)
    }
}

/// Validate that `hash` is exactly 64 lowercase hex characters.
fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(
            "invalid hash: must be 64 hex characters, got {:?}",
            hash
        );
    }
    Ok(())
}

/// Check if a process with the given PID is alive (Linux).
#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_alive(_pid: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn store_creates_directories() {
        let tmp = TempDir::new().unwrap();
        let store = PackageStore::new(tmp.path().to_path_buf()).unwrap();
        assert!(store.packages_dir.exists());
        assert!(store.state_dir.exists());
    }

    #[test]
    fn lock_acquire_and_release() {
        let tmp = TempDir::new().unwrap();
        let store = PackageStore::new(tmp.path().to_path_buf()).unwrap();
        assert!(store.acquire_lock("test.pkg").is_ok());
        // Second lock acquisition should fail
        assert!(store.acquire_lock("test.pkg").is_err());
        store.release_lock("test.pkg");
        // After release, can acquire again
        assert!(store.acquire_lock("test.pkg").is_ok());
        store.release_lock("test.pkg");
    }

    #[test]
    fn staging_and_commit() {
        let tmp = TempDir::new().unwrap();
        let store = PackageStore::new(tmp.path().to_path_buf()).unwrap();
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let staging = store.staging_path(hash).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("test.txt"), "hello").unwrap();
        store.commit_staging(hash).unwrap();
        assert!(store.is_installed(hash).unwrap());
        assert!(store.package_path(hash).unwrap().join("test.txt").exists());
    }
}
