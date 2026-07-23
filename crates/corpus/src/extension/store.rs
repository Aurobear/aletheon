//! Durable, content-addressed extension package store.

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPackageRecord {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub description: String,
    pub hash: String,
    pub file_count: usize,
    pub total_size: u64,
    pub installed_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationRecord {
    pub schema_version: u32,
    pub package_id: String,
    pub enabled: bool,
    pub current: Option<String>,
    pub previous_known_good: Option<String>,
}

/// An exclusive package transaction lock. The lock is always released on drop.
pub struct PackageLockGuard {
    path: PathBuf,
}

impl Drop for PackageLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct PackageStore {
    root: PathBuf,
    packages_dir: PathBuf,
    state_dir: PathBuf,
    staging_dir: PathBuf,
}

impl PackageStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        let packages_dir = root.join("packages");
        let state_dir = root.join("state");
        let staging_dir = root.join("staging");
        for dir in [&packages_dir, &state_dir, &staging_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self { root, packages_dir, state_dir, staging_dir })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn package_path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.packages_dir.join(hash))
    }

    pub fn staging_path(&self, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.staging_dir.join(hash))
    }

    fn state_key(package_id: &str) -> String {
        format!("{:x}", Sha256::digest(package_id.as_bytes()))
    }

    fn package_state_dir(&self, package_id: &str) -> PathBuf {
        self.state_dir.join(Self::state_key(package_id))
    }

    pub fn activation_path(&self, package_id: &str) -> PathBuf {
        self.package_state_dir(package_id).join("activation.json")
    }

    pub fn lock_path(&self, package_id: &str) -> PathBuf {
        self.package_state_dir(package_id).join("transaction.lock")
    }

    pub fn acquire_lock(&self, package_id: &str) -> Result<PackageLockGuard> {
        let lock_path = self.lock_path(package_id);
        let parent = lock_path.parent().context("lock has no parent")?;
        std::fs::create_dir_all(parent)?;
        for attempt in 0..2 {
            match OpenOptions::new().create_new(true).write(true).open(&lock_path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    file.sync_all()?;
                    sync_dir(parent)?;
                    return Ok(PackageLockGuard { path: lock_path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                    let holder = std::fs::read_to_string(&lock_path)
                        .ok()
                        .and_then(|value| value.trim().parse::<i32>().ok());
                    if holder.is_some_and(pid_alive) {
                        bail!("package '{}' is locked by process {}", package_id, holder.unwrap());
                    }
                    std::fs::remove_file(&lock_path)
                        .context("cannot remove stale package lock")?;
                }
                Err(error) => return Err(error).context("cannot acquire package lock"),
            }
        }
        unreachable!()
    }

    pub fn is_installed(&self, hash: &str) -> Result<bool> {
        Ok(self.package_path(hash)?.is_dir())
    }

    /// Atomically commits a staging directory. Repeating an already committed
    /// install is successful and removes an obsolete staging copy.
    pub fn commit_staging(&self, hash: &str) -> Result<()> {
        let staging = self.staging_path(hash)?;
        let destination = self.package_path(hash)?;
        if destination.is_dir() {
            if staging.exists() {
                std::fs::remove_dir_all(staging)?;
            }
            return Ok(());
        }
        std::fs::rename(&staging, &destination)
            .with_context(|| format!("cannot commit staging to {}", destination.display()))?;
        sync_dir(&self.packages_dir)
    }

    pub fn clean_staging(&self, hash: &str) -> Result<()> {
        let path = self.staging_path(hash)?;
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    fn version_record_path(&self, package_id: &str, hash: &str) -> Result<PathBuf> {
        validate_hash(hash)?;
        Ok(self.package_state_dir(package_id).join("versions").join(format!("{hash}.json")))
    }

    pub fn put_installed(&self, record: &InstalledPackageRecord) -> Result<()> {
        validate_hash(&record.hash)?;
        write_json_atomic(&self.version_record_path(&record.id, &record.hash)?, record)
    }

    pub fn get_installed(&self, package_id: &str) -> Result<Vec<InstalledPackageRecord>> {
        let directory = self.package_state_dir(package_id).join("versions");
        read_records(&directory).map(|mut records: Vec<InstalledPackageRecord>| {
            records.sort_by(|a, b| a.installed_at.cmp(&b.installed_at).then(a.hash.cmp(&b.hash)));
            records
        })
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledPackageRecord>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&self.state_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                records.extend(read_records::<InstalledPackageRecord>(
                    &entry.path().join("versions"),
                )?);
            }
        }
        records.sort_by(|a, b| a.id.cmp(&b.id).then(a.version.cmp(&b.version)));
        Ok(records)
    }

    pub fn read_activation(&self, package_id: &str) -> Result<ActivationRecord> {
        let path = self.activation_path(package_id);
        if !path.exists() {
            return Ok(ActivationRecord {
                schema_version: 1,
                package_id: package_id.to_owned(),
                ..ActivationRecord::default()
            });
        }
        read_json(&path)
    }

    pub fn write_activation(&self, record: &ActivationRecord) -> Result<()> {
        write_json_atomic(&self.activation_path(&record.package_id), record)
    }

    pub fn store_receipt(&self, package_id: &str, receipt: &serde_json::Value) -> Result<PathBuf> {
        let directory = self.package_state_dir(package_id).join("receipts");
        let path = directory.join(format!("{}.json", Uuid::new_v4()));
        write_json_atomic(&path, receipt)?;
        Ok(path)
    }

    pub fn remove_state(&self, package_id: &str) -> Result<()> {
        let path = self.package_state_dir(package_id);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
            sync_dir(&self.state_dir)?;
        }
        Ok(())
    }

    pub fn remove_package_if_unreferenced(&self, hash: &str) -> Result<()> {
        validate_hash(hash)?;
        if self.list_installed()?.iter().any(|record| record.hash == hash) {
            return Ok(());
        }
        let path = self.package_path(hash)?;
        if path.exists() {
            std::fs::remove_dir_all(path)?;
            sync_dir(&self.packages_dir)?;
        }
        Ok(())
    }
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid hash: expected 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("state path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path)?)
        .with_context(|| format!("cannot decode {}", path.display()))
}

fn read_records<T: DeserializeOwned>(directory: &Path) -> Result<Vec<T>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().is_some_and(|extension| extension == "json")
        {
            records.push(read_json(&entry.path())?);
        }
    }
    Ok(records)
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

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

    const HASH: &str = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

    #[test]
    fn state_keys_do_not_collide_after_character_replacement() {
        let store = PackageStore::new(TempDir::new().unwrap().path().to_owned()).unwrap();
        assert_ne!(store.activation_path("a.b"), store.activation_path("a/b"));
    }

    #[test]
    fn lock_is_released_by_drop() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        let guard = store.acquire_lock("test.pkg").unwrap();
        assert!(store.acquire_lock("test.pkg").is_err());
        drop(guard);
        assert!(store.acquire_lock("test.pkg").is_ok());
    }

    #[test]
    fn staging_uses_full_hash_and_commit_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        let staging = store.staging_path(HASH).unwrap();
        assert_eq!(staging.file_name().unwrap(), HASH);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("file"), "content").unwrap();
        store.commit_staging(HASH).unwrap();
        store.commit_staging(HASH).unwrap();
        assert!(store.is_installed(HASH).unwrap());
    }

    #[test]
    fn uppercase_hash_is_rejected() {
        assert!(validate_hash(&"A".repeat(64)).is_err());
    }

    #[test]
    fn installed_projection_round_trips() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        let record = InstalledPackageRecord {
            schema_version: 1,
            id: "test.pkg".into(),
            version: "1.0.0".into(),
            description: "test".into(),
            hash: HASH.into(),
            file_count: 1,
            total_size: 7,
            installed_at: "2026-07-23T00:00:00Z".into(),
        };
        store.put_installed(&record).unwrap();
        assert_eq!(store.list_installed().unwrap(), vec![record]);
    }
}
