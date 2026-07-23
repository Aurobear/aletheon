//! Extension lifecycle management service.
//!
//! Handles enable/disable/upgrade/rollback/remove/purge/doctor operations
//! through application-level ports. CLI commands call this; they never
//! manipulate store state directly.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Result of a doctor check on an extension.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionDoctorResult {
    pub id: String,
    pub healthy: bool,
    pub issues: Vec<String>,
}

/// Application service for extension lifecycle management.
pub struct ExtensionManageService;

impl ExtensionManageService {
    /// Enable an installed extension (makes it active).
    pub fn enable(&self, id: &str) -> Result<()> {
        anyhow::bail!("not yet implemented: enable (R5 — upgrade/rollback infra needed)")
    }

    /// Disable an active extension.
    pub fn disable(&self, id: &str) -> Result<()> {
        anyhow::bail!("not yet implemented: disable (R5 — upgrade/rollback infra needed)")
    }

    /// Upgrade an extension to a newer version from a package.
    pub fn upgrade(&self, package_path: &Path) -> Result<()> {
        anyhow::bail!("not yet implemented: upgrade (R5 — upgrade/rollback infra needed)")
    }

    /// Rollback to the previous known-good version.
    pub fn rollback(&self, id: &str) -> Result<()> {
        anyhow::bail!("not yet implemented: rollback (R5 — upgrade/rollback infra needed)")
    }

    /// Remove an extension (deactivate, keep package files).
    pub fn remove(&self, id: &str) -> Result<()> {
        anyhow::bail!("not yet implemented: remove (R5 — upgrade/rollback infra needed)")
    }

    /// Purge an extension (remove package files and all state).
    pub fn purge(&self, id: &str) -> Result<()> {
        anyhow::bail!("not yet implemented: purge (R5 — upgrade/rollback infra needed)")
    }

    /// Run diagnostics on an extension and return structured results.
    pub fn doctor(&self, id: &str) -> Result<ExtensionDoctorResult> {
        // R5 initial: report that the extension exists but detailed health
        // requires runtime wiring from R6.
        Ok(ExtensionDoctorResult {
            id: id.to_string(),
            healthy: true,
            issues: vec![format!("detailed health checks not yet implemented for: {id}")],
        })
    }

    /// Import a legacy extension from the filesystem.
    pub fn import_legacy(&self) -> Result<Vec<String>> {
        anyhow::bail!("not yet implemented: import-legacy (R5)")
    }
}
