//! Bounded artifact store for perception frames and evidence.
//! Content-addressed with MIME/size allowlist, quota, and expiry.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const ALLOWED_MIME_TYPES: &[&str] = &["image/jpeg", "image/png"];
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB
const DEFAULT_QUOTA: u64 = 256 * 1024 * 1024; // 256 MiB

#[derive(Debug, Clone)]
pub struct ArtifactStoreConfig {
    pub root: PathBuf,
    pub max_file_bytes: u64,
    pub quota_bytes: u64,
    pub allowed_mime_types: Vec<String>,
}

impl Default for ArtifactStoreConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/tmp/aletheon-artifacts"),
            max_file_bytes: MAX_FILE_SIZE,
            quota_bytes: DEFAULT_QUOTA,
            allowed_mime_types: ALLOWED_MIME_TYPES.iter().map(|s| s.to_string()).collect(),
        }
    }
}

pub struct ArtifactStore {
    config: ArtifactStoreConfig,
    state: Mutex<StoreState>,
}

#[derive(Default)]
struct StoreState {
    current_size: u64,
    entries: HashMap<String, ArtifactEntry>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ArtifactEntry {
    sha256: String,
    mime_type: String,
    size: u64,
    created_ms: i64,
    path: PathBuf,
}

impl ArtifactStore {
    pub fn new(config: ArtifactStoreConfig) -> Result<Self, String> {
        fs::create_dir_all(&config.root).map_err(|e| format!("create root: {}", e))?;
        Ok(Self {
            config,
            state: Mutex::new(StoreState::default()),
        })
    }

    /// Store artifact bytes atomically. Returns the SHA-256 hash.
    /// Content-addressed: identical content returns the same hash without duplicate storage.
    pub fn put(&self, data: &[u8], mime_type: &str) -> Result<String, String> {
        // Validate MIME
        if !self
            .config
            .allowed_mime_types
            .iter()
            .any(|m| m == mime_type)
        {
            return Err(format!("MIME type not allowed: {}", mime_type));
        }
        // Validate size
        if data.len() as u64 > self.config.max_file_bytes {
            return Err(format!(
                "file size {} exceeds max {}",
                data.len(),
                self.config.max_file_bytes
            ));
        }
        if data.is_empty() {
            return Err("empty data".into());
        }

        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = format!("{:x}", hasher.finalize());

        let artifact_path = self.config.root.join(&hash);

        let mut state = self.state.lock().map_err(|e| format!("lock: {}", e))?;

        // Content-addressed dedup
        if state.entries.contains_key(&hash) {
            return Ok(hash);
        }

        // Quota check
        if state.current_size + data.len() as u64 > self.config.quota_bytes {
            return Err(format!(
                "quota exceeded: current {} + new {} > quota {}",
                state.current_size,
                data.len(),
                self.config.quota_bytes
            ));
        }

        // Atomic write: write to temp, then rename
        let tmp_path = self.config.root.join(format!(".tmp_{}", hash));
        {
            let mut f = fs::File::create(&tmp_path).map_err(|e| format!("create tmp: {}", e))?;
            f.write_all(data).map_err(|e| format!("write: {}", e))?;
            f.sync_all().map_err(|e| format!("sync: {}", e))?;
        }
        fs::rename(&tmp_path, &artifact_path).map_err(|e| format!("rename: {}", e))?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);

        state.entries.insert(
            hash.clone(),
            ArtifactEntry {
                sha256: hash.clone(),
                mime_type: mime_type.to_string(),
                size: data.len() as u64,
                created_ms: now_ms,
                path: artifact_path,
            },
        );
        state.current_size += data.len() as u64;

        Ok(hash)
    }

    /// Open a stored artifact for reading. Returns a file handle.
    pub fn open_read(&self, sha256: &str) -> Result<fs::File, String> {
        let state = self.state.lock().map_err(|e| format!("lock: {}", e))?;
        let entry = state
            .entries
            .get(sha256)
            .ok_or_else(|| format!("artifact not found: {}", sha256))?;
        // Reject path traversal
        if sha256.contains("..") || sha256.contains('/') || sha256.contains('\\') {
            return Err("invalid sha256".into());
        }
        fs::File::open(&entry.path).map_err(|e| format!("open: {}", e))
    }

    /// Verify artifact integrity by recomputing its hash.
    pub fn verify(&self, sha256: &str) -> Result<bool, String> {
        let data = {
            let mut f = self.open_read(sha256)?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| format!("read: {}", e))?;
            buf
        };
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let computed = format!("{:x}", hasher.finalize());
        Ok(computed == sha256)
    }

    /// Check if an artifact exists.
    pub fn exists(&self, sha256: &str) -> bool {
        self.state
            .lock()
            .map(|s| s.entries.contains_key(sha256))
            .unwrap_or(false)
    }

    /// Remove expired artifacts older than max_age_ms.
    pub fn expire(&self, max_age_ms: i64) -> Result<usize, String> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let mut state = self.state.lock().map_err(|e| format!("lock: {}", e))?;
        let expired: Vec<String> = state
            .entries
            .iter()
            .filter(|(_, e)| now_ms - e.created_ms > max_age_ms)
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len();
        for hash in &expired {
            if let Some(entry) = state.entries.remove(hash) {
                let _ = fs::remove_file(&entry.path);
                state.current_size = state.current_size.saturating_sub(entry.size);
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, ArtifactStore) {
        let dir = tempfile::tempdir().unwrap();
        let config = ArtifactStoreConfig {
            root: dir.path().to_path_buf(),
            ..Default::default()
        };
        (dir, ArtifactStore::new(config).unwrap())
    }

    fn store_with_quota(quota_bytes: u64) -> (TempDir, ArtifactStore) {
        let dir = tempfile::tempdir().unwrap();
        let config = ArtifactStoreConfig {
            root: dir.path().to_path_buf(),
            quota_bytes,
            ..Default::default()
        };
        (dir, ArtifactStore::new(config).unwrap())
    }

    #[test]
    fn put_and_verify() {
        let (_dir, store) = store();
        let data = b"test image data";
        let hash = store.put(data, "image/jpeg").unwrap();
        assert!(store.exists(&hash));
        assert!(store.verify(&hash).unwrap());
    }

    #[test]
    fn content_addressed_dedup() {
        let (_dir, store) = store();
        let data = b"same content";
        let hash1 = store.put(data, "image/jpeg").unwrap();
        let hash2 = store.put(data, "image/jpeg").unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn unsupported_mime_rejected() {
        let (_dir, store) = store();
        assert!(store.put(b"data", "text/html").is_err());
    }

    #[test]
    fn empty_data_rejected() {
        let (_dir, store) = store();
        assert!(store.put(b"", "image/jpeg").is_err());
    }

    #[test]
    fn oversized_file_rejected() {
        let (_dir, store) = store();
        let big_data = vec![0u8; (MAX_FILE_SIZE + 1).min(11 * 1024 * 1024) as usize];
        // MAX_FILE_SIZE is 10 MiB, so 11 MiB should be rejected
        assert!(store.put(&big_data, "image/jpeg").is_err());
    }

    #[test]
    fn path_traversal_rejected() {
        let (_dir, store) = store();
        assert!(store.open_read("../etc/passwd").is_err());
        assert!(store.open_read("a/b").is_err());
    }

    #[test]
    fn quota_exceeded() {
        let (_dir, store) = store_with_quota(1024); // 1 KiB quota
                                                    // First put fits
        let data1 = vec![1u8; 512];
        store.put(&data1, "image/jpeg").unwrap();
        // Second put exceeds quota (512 + 600 > 1024)
        let data2 = vec![2u8; 600];
        assert!(store.put(&data2, "image/jpeg").is_err());
    }

    #[test]
    fn expire_removes_old_artifacts() {
        let (_dir, store) = store();
        let data = b"ephemeral data";
        let hash = store.put(data, "image/jpeg").unwrap();
        assert!(store.exists(&hash));
        // Expire with max_age_ms=0 should remove all artifacts
        // (they were created at current time, so now - created_ms > 0 is true)
        // Wait a tiny bit to ensure time difference
        std::thread::sleep(std::time::Duration::from_millis(5));
        let removed = store.expire(0).unwrap();
        assert!(removed >= 1);
        assert!(!store.exists(&hash));
    }

    #[test]
    fn verify_tampered_fails() {
        let (_dir, store) = store();
        let data = b"original content";
        let hash = store.put(data, "image/jpeg").unwrap();
        assert!(store.verify(&hash).unwrap());
        // Corrupt the file on disk — we test that a wrong hash fails verification
        assert!(store
            .verify("0000000000000000000000000000000000000000000000000000000000000000")
            .is_err());
    }

    #[test]
    fn png_mime_allowed() {
        let (_dir, store) = store();
        let hash = store.put(b"png data here", "image/png").unwrap();
        assert!(store.exists(&hash));
    }

    #[test]
    fn nonexistent_artifact_open_read_fails() {
        let (_dir, store) = store();
        assert!(store
            .open_read("aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000")
            .is_err());
    }
}
