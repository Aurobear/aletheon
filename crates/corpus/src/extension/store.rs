//! Durable, content-addressed extension package store.

use anyhow::{bail, Context, Result};
use fabric::types::extension_package::{AssetRef, PermissionRequestSet};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    #[serde(default)]
    pub assets: Vec<AssetRef>,
    #[serde(default)]
    pub requested_permissions: PermissionRequestSet,
    #[serde(default)]
    pub source: PackageSourceRecord,
    #[serde(default)]
    pub workspace_trust: Option<WorkspaceTrustRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageSourceRecord {
    #[default]
    LocalArchive,
    Workspace,
    LegacyImport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTrustRecord {
    pub actor: String,
    pub approved_at: String,
    pub package_hash: String,
}

/// Generic extension evidence envelope. It deliberately lives below Metacog:
/// observers may consume it, but cannot use it to mutate extension state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionEvidenceEvent {
    pub schema_version: u32,
    pub event_type: String,
    pub correlation_id: String,
    pub package_id: String,
    pub package_version: Option<String>,
    pub result: String,
    pub evidence_references: Vec<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyMigrationReport {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub compatibility_reads: BTreeMap<String, u64>,
    #[serde(default)]
    pub imported_package_hashes: Vec<String>,
    pub remaining_candidates: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionApprovalRecord {
    pub actor: String,
    pub approved_at: String,
    pub permissions: PermissionRequestSet,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationRecord {
    pub schema_version: u32,
    pub package_id: String,
    pub enabled: bool,
    pub current: Option<String>,
    pub previous_known_good: Option<String>,
    #[serde(default)]
    pub granted_permissions: PermissionRequestSet,
    #[serde(default)]
    pub permission_approval: Option<PermissionApprovalRecord>,
    #[serde(default)]
    pub activated_assets: Vec<String>,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub quarantine_reason: Option<String>,
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
    pub fn configured_user_root() -> PathBuf {
        std::env::var_os("ALETHEON_EXTENSION_STORE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("aletheon")
                    .join("extensions")
            })
    }

    pub fn new(root: PathBuf) -> Result<Self> {
        let packages_dir = root.join("packages");
        let state_dir = root.join("state");
        let staging_dir = root.join("staging");
        for dir in [&packages_dir, &state_dir, &staging_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self {
            root,
            packages_dir,
            state_dir,
            staging_dir,
        })
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
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
            {
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
                        bail!(
                            "package '{}' is locked by process {}",
                            package_id,
                            holder.unwrap()
                        );
                    }
                    std::fs::remove_file(&lock_path).context("cannot remove stale package lock")?;
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
        Ok(self
            .package_state_dir(package_id)
            .join("versions")
            .join(format!("{hash}.json")))
    }

    pub fn put_installed(&self, record: &InstalledPackageRecord) -> Result<()> {
        validate_hash(&record.hash)?;
        write_json_atomic(&self.version_record_path(&record.id, &record.hash)?, record)
    }

    pub fn get_installed(&self, package_id: &str) -> Result<Vec<InstalledPackageRecord>> {
        let directory = self.package_state_dir(package_id).join("versions");
        read_records(&directory).map(|mut records: Vec<InstalledPackageRecord>| {
            records.sort_by(|a, b| {
                a.installed_at
                    .cmp(&b.installed_at)
                    .then(a.hash.cmp(&b.hash))
            });
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

    /// Append a bounded, structured observation for Metacog and other generic
    /// evidence consumers. State mutation remains exclusively in lifecycle
    /// services; this stream is observation-only.
    pub fn append_evidence(&self, event: &ExtensionEvidenceEvent) -> Result<()> {
        anyhow::ensure!(event.schema_version == 1, "unsupported evidence schema");
        anyhow::ensure!(
            !event.event_type.trim().is_empty(),
            "event type is required"
        );
        anyhow::ensure!(
            !event.correlation_id.trim().is_empty(),
            "correlation ID is required"
        );
        anyhow::ensure!(event.result.len() <= 64, "event result is too long");
        anyhow::ensure!(
            event.evidence_references.len() <= 16
                && event
                    .evidence_references
                    .iter()
                    .all(|reference| reference.len() <= 256),
            "evidence references exceed persistence bounds"
        );
        let _lock = self.acquire_lock("__extension_evidence__")?;
        let path = self.root.join("metacog-events.jsonl");
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(&line)?;
        file.sync_all()?;
        sync_dir(&self.root)?;
        Ok(())
    }

    pub fn record_legacy_compatibility_read(&self, category: &str) -> Result<()> {
        anyhow::ensure!(
            !category.is_empty()
                && category.len() <= 64
                && category
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "invalid compatibility read category"
        );
        self.update_migration_report(|report| {
            *report
                .compatibility_reads
                .entry(category.to_owned())
                .or_default() += 1;
        })
    }

    pub fn record_legacy_import(&self, hash: &str) -> Result<()> {
        validate_hash(hash)?;
        self.update_migration_report(|report| {
            if !report
                .imported_package_hashes
                .iter()
                .any(|value| value == hash)
            {
                report.imported_package_hashes.push(hash.to_owned());
                report.imported_package_hashes.sort();
            }
        })
    }

    pub fn set_remaining_legacy_candidates(&self, count: u64) -> Result<()> {
        self.update_migration_report(|report| report.remaining_candidates = count)
    }

    pub fn legacy_migration_report(&self) -> Result<LegacyMigrationReport> {
        let path = self.root.join("legacy-migration-report.json");
        if path.exists() {
            read_json(&path)
        } else {
            Ok(LegacyMigrationReport {
                schema_version: 1,
                generated_at: chrono::Utc::now().to_rfc3339(),
                ..LegacyMigrationReport::default()
            })
        }
    }

    fn update_migration_report(
        &self,
        update: impl FnOnce(&mut LegacyMigrationReport),
    ) -> Result<()> {
        let _lock = self.acquire_lock("__legacy_migration__")?;
        let mut report = self.legacy_migration_report()?;
        report.schema_version = 1;
        update(&mut report);
        report.generated_at = chrono::Utc::now().to_rfc3339();
        write_json_atomic(&self.root.join("legacy-migration-report.json"), &report)
    }

    /// Rebuild the installed projection from durable install receipts.
    ///
    /// This is intentionally explicit: callers may audit the receipts before
    /// replacing a damaged projection. Unknown receipt schemas and operations
    /// are ignored, while malformed install records fail closed.
    pub fn replay_install_receipts(&self) -> Result<usize> {
        let mut rebuilt = 0;
        for state in std::fs::read_dir(&self.state_dir)? {
            let state = state?;
            let receipts = state.path().join("receipts");
            if !receipts.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(receipts)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let value: serde_json::Value = read_json(&entry.path())?;
                if value.get("schema_version").and_then(|value| value.as_u64()) != Some(1)
                    || value.get("operation").and_then(|value| value.as_str()) != Some("install")
                {
                    continue;
                }
                let record: InstalledPackageRecord = serde_json::from_value(
                    value
                        .get("record")
                        .cloned()
                        .context("install receipt is missing its projection record")?,
                )
                .context("install receipt contains an invalid projection record")?;
                anyhow::ensure!(
                    self.is_installed(&record.hash)?,
                    "install receipt points to missing package content: {}",
                    record.hash
                );
                self.put_installed(&record)?;
                rebuilt += 1;
            }
        }
        Ok(rebuilt)
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
        if self
            .list_installed()?
            .iter()
            .any(|record| record.hash == hash)
        {
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
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
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
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
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
            assets: Vec::new(),
            requested_permissions: PermissionRequestSet::default(),
            source: PackageSourceRecord::LocalArchive,
            workspace_trust: None,
        };
        store.put_installed(&record).unwrap();
        assert_eq!(store.list_installed().unwrap(), vec![record]);
    }

    #[test]
    fn install_receipt_replays_a_missing_projection() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        std::fs::create_dir_all(store.package_path(HASH).unwrap()).unwrap();
        let record = InstalledPackageRecord {
            schema_version: 1,
            id: "test.replay".into(),
            version: "1.0.0".into(),
            description: "test".into(),
            hash: HASH.into(),
            file_count: 1,
            total_size: 7,
            installed_at: "2026-07-23T00:00:00Z".into(),
            assets: Vec::new(),
            requested_permissions: PermissionRequestSet::default(),
            source: PackageSourceRecord::LocalArchive,
            workspace_trust: None,
        };
        store
            .store_receipt(
                &record.id,
                &serde_json::json!({
                    "schema_version": 1,
                    "operation": "install",
                    "record": record,
                }),
            )
            .unwrap();
        assert!(store.list_installed().unwrap().is_empty());
        assert_eq!(store.replay_install_receipts().unwrap(), 1);
        assert_eq!(store.list_installed().unwrap(), vec![record]);
    }

    #[test]
    fn generic_evidence_is_durable_bounded_jsonl() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        let event = ExtensionEvidenceEvent {
            schema_version: 1,
            event_type: "activation_failure".into(),
            correlation_id: "correlation-1".into(),
            package_id: "test.pkg".into(),
            package_version: Some("1.0.0".into()),
            result: "failed".into(),
            evidence_references: vec![format!("package:sha256:{HASH}")],
            occurred_at: "2026-07-24T00:00:00Z".into(),
        };
        store.append_evidence(&event).unwrap();
        let line = std::fs::read_to_string(temp.path().join("metacog-events.jsonl")).unwrap();
        assert_eq!(
            serde_json::from_str::<ExtensionEvidenceEvent>(line.trim()).unwrap(),
            event
        );

        let mut oversized = event;
        oversized.result = "x".repeat(65);
        assert!(store.append_evidence(&oversized).is_err());
    }

    #[test]
    fn legacy_migration_report_tracks_reads_imports_and_remaining_work() {
        let temp = TempDir::new().unwrap();
        let store = PackageStore::new(temp.path().to_owned()).unwrap();
        store
            .record_legacy_compatibility_read("legacy_filesystem_archive")
            .unwrap();
        store.record_legacy_import(HASH).unwrap();
        store.set_remaining_legacy_candidates(2).unwrap();

        let report = store.legacy_migration_report().unwrap();
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.compatibility_reads["legacy_filesystem_archive"], 1);
        assert_eq!(report.imported_package_hashes, vec![HASH]);
        assert_eq!(report.remaining_candidates, 2);
        assert!(store
            .record_legacy_compatibility_read("../invalid")
            .is_err());
    }
}
