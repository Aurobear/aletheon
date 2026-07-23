//! Security validation for extension packages.
//!
//! Validates archive entries before extraction: path traversal detection,
//! size limits, file count limits, directory depth limits, and checksum
//! verification.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Maximum single file size: 100 MB.
pub const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Maximum total extracted size: 500 MB.
pub const MAX_TOTAL_SIZE: u64 = 500 * 1024 * 1024;

/// Maximum number of files in the archive.
pub const MAX_FILE_COUNT: usize = 10_000;

/// Maximum directory nesting depth.
pub const MAX_DIR_DEPTH: usize = 20;

/// Validate a path within an archive entry.
///
/// Rejects: absolute paths, parent directory traversal (`..`), empty paths.
pub fn validate_entry_path(entry_path: &Path) -> Result<()> {
    let path_str = entry_path.to_string_lossy();

    if path_str.is_empty() {
        bail!("empty entry path");
    }

    if entry_path.is_absolute() {
        bail!("absolute path forbidden: {}", path_str);
    }

    for component in entry_path.components() {
        match component {
            std::path::Component::ParentDir => {
                bail!("parent directory traversal forbidden: {}", path_str);
            }
            std::path::Component::RootDir => {
                bail!("absolute path forbidden: {}", path_str);
            }
            _ => {}
        }
    }

    // Check depth
    let depth = entry_path.components().count();
    if depth > MAX_DIR_DEPTH {
        bail!(
            "directory depth {} exceeds maximum {}: {}",
            depth,
            MAX_DIR_DEPTH,
            path_str
        );
    }

    Ok(())
}

/// Check file size against limits.
pub fn validate_file_size(size: u64, path: &str) -> Result<()> {
    if size > MAX_FILE_SIZE {
        bail!(
            "file size {} exceeds maximum {}: {}",
            size,
            MAX_FILE_SIZE,
            path
        );
    }
    Ok(())
}

/// Check running total against maximum.
pub fn validate_total_size(total: u64) -> Result<()> {
    if total > MAX_TOTAL_SIZE {
        bail!(
            "total extracted size {} exceeds maximum {}",
            total,
            MAX_TOTAL_SIZE
        );
    }
    Ok(())
}

/// Check file count against maximum.
pub fn validate_file_count(count: usize) -> Result<()> {
    if count > MAX_FILE_COUNT {
        bail!(
            "file count {} exceeds maximum {}",
            count,
            MAX_FILE_COUNT
        );
    }
    Ok(())
}

/// Verify a file's SHA-256 hash against the expected value.
pub fn verify_hash(data: &[u8], expected_hex: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(data));
    if actual != expected_hex {
        bail!(
            "checksum mismatch: expected {}, got {}",
            expected_hex,
            actual
        );
    }
    Ok(())
}

/// Verify all checksums in a map against file data.
/// `files` maps relative paths to their raw bytes.
pub fn verify_all_checksums(
    files: &HashMap<String, Vec<u8>>,
    checksums: &HashMap<String, String>,
) -> Result<()> {
    for (path, expected) in checksums {
        let data = files
            .get(path)
            .with_context(|| format!("file not found in archive: {}", path))?;
        verify_hash(data, expected)
            .with_context(|| format!("checksum verification failed for: {}", path))?;
    }

    // Verify no extra undeclared files exist (except checksums.sha256 itself)
    for path in files.keys() {
        if path == "checksums.sha256" {
            continue;
        }
        if !checksums.contains_key(path) {
            bail!("file not declared in checksums.sha256: {}", path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_absolute_path() {
        assert!(validate_entry_path(Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn reject_parent_traversal() {
        assert!(validate_entry_path(Path::new("../etc/passwd")).is_err());
        assert!(validate_entry_path(Path::new("foo/../../bar")).is_err());
    }

    #[test]
    fn accept_normal_relative_path() {
        assert!(validate_entry_path(Path::new("assets/skills/demo/SKILL.md")).is_ok());
        assert!(validate_entry_path(Path::new("extension.toml")).is_ok());
    }

    #[test]
    fn reject_empty_path() {
        assert!(validate_entry_path(Path::new("")).is_err());
    }

    #[test]
    fn reject_too_deep() {
        let deep: String = (0..=MAX_DIR_DEPTH)
            .map(|i| format!("dir{}", i))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_entry_path(Path::new(&deep)).is_err());
    }

    #[test]
    fn verify_hash_passes() {
        let data = b"hello world";
        let hash = format!("{:x}", Sha256::digest(data));
        assert!(verify_hash(data, &hash).is_ok());
    }

    #[test]
    fn verify_hash_fails() {
        assert!(verify_hash(b"hello world", "deadbeef").is_err());
    }

    #[test]
    fn reject_oversized_file() {
        assert!(validate_file_size(MAX_FILE_SIZE + 1, "big.bin").is_err());
        assert!(validate_file_size(MAX_FILE_SIZE, "ok.bin").is_ok());
    }

    #[test]
    fn reject_too_many_files() {
        assert!(validate_file_count(MAX_FILE_COUNT + 1).is_err());
        assert!(validate_file_count(MAX_FILE_COUNT).is_ok());
    }
}
