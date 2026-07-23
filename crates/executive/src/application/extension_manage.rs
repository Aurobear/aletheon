//! Transactional extension lifecycle management.

use anyhow::{Context, Result};
use corpus::extension::store::{ActivationRecord, PackageStore};
use std::path::Path;
use std::sync::Arc;

use super::extension_install::ExtensionInstallService;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionDoctorResult {
    pub id: String,
    pub healthy: bool,
    pub issues: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::extension::store::InstalledPackageRecord;
    use tempfile::TempDir;

    const HASH_A: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn record(hash: &str, version: &str, installed_at: &str) -> InstalledPackageRecord {
        InstalledPackageRecord {
            schema_version: 1,
            id: "test.pkg".into(),
            version: version.into(),
            description: "test".into(),
            hash: hash.into(),
            file_count: 1,
            total_size: 1,
            installed_at: installed_at.into(),
        }
    }

    #[test]
    fn lifecycle_updates_durable_state_and_rolls_back() {
        let temp = TempDir::new().unwrap();
        let service = ExtensionManageService::new(temp.path()).unwrap();
        for (hash, version, time) in [
            (HASH_A, "1.0.0", "2026-07-23T00:00:00Z"),
            (HASH_B, "2.0.0", "2026-07-23T01:00:00Z"),
        ] {
            std::fs::create_dir_all(service.store.package_path(hash).unwrap()).unwrap();
            service.store.put_installed(&record(hash, version, time)).unwrap();
        }
        service.store.write_activation(&ActivationRecord {
            schema_version: 1,
            package_id: "test.pkg".into(),
            enabled: true,
            current: Some(HASH_B.into()),
            previous_known_good: Some(HASH_A.into()),
        }).unwrap();

        service.disable("test.pkg").unwrap();
        assert!(!service.store.read_activation("test.pkg").unwrap().enabled);
        service.rollback("test.pkg").unwrap();
        let state = service.store.read_activation("test.pkg").unwrap();
        assert!(state.enabled);
        assert_eq!(state.current.as_deref(), Some(HASH_A));
        assert_eq!(state.previous_known_good.as_deref(), Some(HASH_B));
        assert!(service.doctor("test.pkg").unwrap().healthy);
    }

    #[test]
    fn purge_removes_state_and_unreferenced_content() {
        let temp = TempDir::new().unwrap();
        let service = ExtensionManageService::new(temp.path()).unwrap();
        std::fs::create_dir_all(service.store.package_path(HASH_A).unwrap()).unwrap();
        service.store.put_installed(
            &record(HASH_A, "1.0.0", "2026-07-23T00:00:00Z"),
        ).unwrap();
        service.purge("test.pkg").unwrap();
        assert!(service.store.get_installed("test.pkg").unwrap().is_empty());
        assert!(!service.store.is_installed(HASH_A).unwrap());
    }
}

pub struct ExtensionManageService {
    store: Arc<PackageStore>,
    installer: ExtensionInstallService,
}

impl ExtensionManageService {
    pub fn new(store_root: &Path) -> Result<Self> {
        Ok(Self {
            store: Arc::new(PackageStore::new(store_root.to_owned())?),
            installer: ExtensionInstallService::new(store_root)?,
        })
    }

    pub fn enable(&self, id: &str) -> Result<()> {
        let _lock = self.store.acquire_lock(id)?;
        let versions = self.store.get_installed(id)?;
        let candidate = versions.last().context("extension is not installed")?;
        anyhow::ensure!(
            self.store.is_installed(&candidate.hash)?,
            "installed projection points to missing package {}",
            candidate.hash
        );
        let mut activation = self.store.read_activation(id)?;
        if activation.current.as_deref() != Some(&candidate.hash) {
            activation.previous_known_good = activation.current.take();
            activation.current = Some(candidate.hash.clone());
        }
        activation.enabled = true;
        self.store.write_activation(&activation)?;
        self.receipt(id, "enable", serde_json::json!({"hash": candidate.hash}))
    }

    pub fn disable(&self, id: &str) -> Result<()> {
        let _lock = self.store.acquire_lock(id)?;
        let mut activation = self.store.read_activation(id)?;
        anyhow::ensure!(activation.current.is_some(), "extension '{id}' has never been enabled");
        activation.enabled = false;
        self.store.write_activation(&activation)?;
        self.receipt(id, "disable", serde_json::json!({}))
    }

    pub fn upgrade(&self, package_path: &Path) -> Result<()> {
        let inspection = self.installer.inspect(package_path)?;
        let id = inspection.manifest.package.id.0.clone();
        let old = self.store.read_activation(&id)?;
        let hash = self.installer.install(package_path)?;
        let _lock = self.store.acquire_lock(&id)?;
        let activation = ActivationRecord {
            schema_version: 1,
            package_id: id.clone(),
            enabled: old.enabled,
            current: Some(hash.clone()),
            previous_known_good: old.current.filter(|value| value != &hash),
        };
        self.store.write_activation(&activation)?;
        self.receipt(
            &id,
            "upgrade",
            serde_json::json!({
                "current": activation.current,
                "previous_known_good": activation.previous_known_good,
            }),
        )
    }

    pub fn rollback(&self, id: &str) -> Result<()> {
        let _lock = self.store.acquire_lock(id)?;
        let mut activation = self.store.read_activation(id)?;
        let previous = activation
            .previous_known_good
            .take()
            .context("no previous known-good version is available")?;
        anyhow::ensure!(
            self.store.is_installed(&previous)?,
            "previous known-good package is missing"
        );
        activation.previous_known_good = activation.current.replace(previous);
        activation.enabled = true;
        self.store.write_activation(&activation)?;
        self.receipt(
            id,
            "rollback",
            serde_json::json!({"current": activation.current}),
        )
    }

    /// Deactivate an extension while retaining packages and history.
    pub fn remove(&self, id: &str) -> Result<()> {
        self.disable(id)?;
        self.receipt(id, "remove", serde_json::json!({"packages_retained": true}))
    }

    pub fn purge(&self, id: &str) -> Result<()> {
        let _lock = self.store.acquire_lock(id)?;
        let hashes: Vec<_> = self
            .store
            .get_installed(id)?
            .into_iter()
            .map(|record| record.hash)
            .collect();
        anyhow::ensure!(!hashes.is_empty(), "extension '{id}' is not installed");
        // Persist the terminal receipt outside the soon-to-be-removed package
        // state so the operation remains auditable.
        let audit_id = format!("purged:{id}");
        self.receipt(
            &audit_id,
            "purge",
            serde_json::json!({"package_id": id, "hashes": hashes}),
        )?;
        self.store.remove_state(id)?;
        for hash in hashes {
            self.store.remove_package_if_unreferenced(&hash)?;
        }
        Ok(())
    }

    pub fn doctor(&self, id: &str) -> Result<ExtensionDoctorResult> {
        let records = self.store.get_installed(id)?;
        let activation = self.store.read_activation(id)?;
        let mut issues = Vec::new();
        if records.is_empty() {
            issues.push("no installed package record".to_owned());
        }
        for record in &records {
            if !self.store.is_installed(&record.hash)? {
                issues.push(format!("package content is missing: {}", record.hash));
            }
        }
        if let Some(current) = &activation.current {
            if !records.iter().any(|record| &record.hash == current) {
                issues.push(format!("active hash has no installed record: {current}"));
            }
        }
        Ok(ExtensionDoctorResult {
            id: id.to_owned(),
            healthy: issues.is_empty(),
            issues,
        })
    }

    /// Import all package archives from a legacy staging directory. Files are
    /// inspected and installed through the same application service.
    pub fn import_legacy(&self, legacy_root: &Path) -> Result<Vec<String>> {
        if !legacy_root.exists() {
            return Ok(Vec::new());
        }
        let mut imported = Vec::new();
        for entry in std::fs::read_dir(legacy_root)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if path.is_file() && (name.ends_with(".tar.gz") || name.ends_with(".tgz")) {
                imported.push(self.installer.install(&path)?);
            }
        }
        Ok(imported)
    }

    fn receipt(&self, id: &str, operation: &str, details: serde_json::Value) -> Result<()> {
        self.store.store_receipt(
            id,
            &serde_json::json!({
                "schema_version": 1,
                "operation": operation,
                "package_id": id,
                "completed_at": chrono::Utc::now().to_rfc3339(),
                "details": details,
            }),
        )?;
        Ok(())
    }
}
