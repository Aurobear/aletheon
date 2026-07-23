//! Extension installation application service.
//!
//! Owns the package store and inspector, provides the single install
//! entry point that CLI and future RPC handlers call.

use anyhow::{Context, Result};
use corpus::extension::inspector;
use corpus::extension::manifest;
use corpus::extension::store::PackageStore;
use std::path::Path;
use std::sync::Arc;

/// Installed package record (returned by list/show).
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstalledPackage {
    pub id: String,
    pub version: String,
    pub description: String,
    pub hash: String,
    pub file_count: usize,
    pub total_size: u64,
}

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
        let hash = &result.package_hash;

        // Verify no hash injection
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("invalid package hash");
        }

        let pkg_id = &result.manifest.package.id.0;

        // Acquire lock
        self.store.acquire_lock(pkg_id)?;

        // Commit staging to final location
        let staging = self.store.staging_path(hash)?;
        if !staging.exists() {
            inspector::extract_to_staging(package_path, &staging)?;
        }
        self.store.commit_staging(hash)?;

        // Write receipt
        let receipt = serde_json::json!({
            "schema_version": 1,
            "package_id": pkg_id,
            "version": result.manifest.package.version.0,
            "hash": hash,
            "file_count": result.file_count,
            "total_size": result.total_size,
        });
        self.store.store_receipt(pkg_id, &receipt.to_string())?;

        // Release lock
        self.store.release_lock(pkg_id);

        Ok(hash.clone())
    }

    /// List installed packages.
    pub fn list(&self) -> Result<Vec<String>> {
        self.store.list_installed()
    }
}
