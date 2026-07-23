//! Extension installation application service.
//!
//! Owns the package store and inspector, provides the single install
//! entry point that CLI and future RPC handlers call.

use anyhow::{Context, Result};
use corpus::extension::inspector;
use corpus::extension::store::{InstalledPackageRecord, PackageStore};
use std::path::Path;
use std::sync::Arc;

/// Installed package record (returned by list/show).
/// Application service for extension installation.
/// CLI and RPC handlers call this — they never touch the store directly.
pub struct ExtensionInstallService {
    store: Arc<PackageStore>,
}

impl ExtensionInstallService {
    pub fn new(store_root: &Path) -> Result<Self> {
        let store = PackageStore::new(store_root.to_path_buf())
            .context("failed to open package store")?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Inspect a package without installing.
    pub fn inspect(&self, package_path: &Path) -> Result<inspector::InspectionResult> {
        inspector::inspect_package(package_path)
    }

    /// Install a package: validate, stage, atomically commit, persist receipt.
    pub fn install(&self, package_path: &Path) -> Result<String> {
        let result = inspector::inspect_package(package_path)?;
        let hash = result.package_hash.as_str();
        let pkg_id = &result.manifest.package.id.0;
        let _lock = self.store.acquire_lock(pkg_id)?;

        // The content-addressed commit is idempotent. Extraction failures and
        // interrupted candidates are cleaned before returning.
        let staging = self.store.staging_path(hash)?;
        if !self.store.is_installed(hash)? {
            self.store.clean_staging(hash)?;
            if let Err(error) = inspector::extract_to_staging(package_path, &staging) {
                let _ = self.store.clean_staging(hash);
                return Err(error);
            }
            if let Err(error) = self.store.commit_staging(hash) {
                let _ = self.store.clean_staging(hash);
                return Err(error);
            }
        }

        let record = InstalledPackageRecord {
            schema_version: 1,
            id: pkg_id.clone(),
            version: result.manifest.package.version.0.clone(),
            description: result.manifest.package.description.clone(),
            hash: hash.to_owned(),
            file_count: result.file_count,
            total_size: result.total_size,
            installed_at: chrono::Utc::now().to_rfc3339(),
        };
        // The record filename is the content hash, so a repeated install
        // replaces the same projection rather than creating duplicates.
        self.store.put_installed(&record)?;
        let receipt = serde_json::json!({
            "schema_version": 1,
            "operation": "install",
            "receipt_id": uuid::Uuid::new_v4(),
            "package_id": pkg_id,
            "version": record.version,
            "hash": hash,
            "file_count": result.file_count,
            "total_size": result.total_size,
            "completed_at": chrono::Utc::now().to_rfc3339(),
        });
        self.store.store_receipt(pkg_id, &receipt)?;

        Ok(hash.to_owned())
    }

    /// List installed packages.
    pub fn list(&self) -> Result<Vec<InstalledPackageRecord>> {
        self.store.list_installed()
    }

    pub fn show(&self, id: &str) -> Result<Vec<InstalledPackageRecord>> {
        let records = self.store.get_installed(id)?;
        anyhow::ensure!(!records.is_empty(), "extension '{id}' is not installed");
        Ok(records)
    }
}
