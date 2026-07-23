//! Extension installation application service.
//!
//! Owns the package store and inspector, provides the single install
//! entry point that CLI and future RPC handlers call.

use anyhow::{Context, Result};
use corpus::extension::inspector;
use corpus::extension::store::{
    ExtensionEvidenceEvent, InstalledPackageRecord, PackageSourceRecord, PackageStore,
    WorkspaceTrustRecord,
};
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
        self.install_with_options(package_path, None, None)
    }

    /// Install an archive discovered by the legacy importer.
    pub fn install_legacy(&self, package_path: &Path) -> Result<String> {
        self.install_with_options(
            package_path,
            Some(PackageSourceRecord::LegacyImport),
            None,
        )
    }

    /// Install a package, requiring an explicit actor when the archive comes
    /// from a workspace-local `.aletheon/extensions` directory.
    pub fn install_with_workspace_trust(
        &self,
        package_path: &Path,
        actor: Option<&str>,
    ) -> Result<String> {
        self.install_with_options(package_path, None, actor)
    }

    fn install_with_options(
        &self,
        package_path: &Path,
        source_override: Option<PackageSourceRecord>,
        workspace_actor: Option<&str>,
    ) -> Result<String> {
        let result = match inspector::inspect_package(package_path) {
            Ok(result) => result,
            Err(error) => {
                self.publish_evidence(
                    "package_validation_failure",
                    "unknown",
                    None,
                    "failed",
                    vec!["package:inspection".into()],
                )?;
                return Err(error);
            }
        };
        let hash = result.package_hash.as_str();
        let pkg_id = &result.manifest.package.id.0;
        let source = source_override.unwrap_or_else(|| {
            if is_workspace_extension_path(package_path) {
                PackageSourceRecord::Workspace
            } else {
                PackageSourceRecord::LocalArchive
            }
        });
        let workspace_trust = match source {
            PackageSourceRecord::Workspace => {
                let actor = workspace_actor
                    .filter(|value| !value.trim().is_empty())
                    .context(
                        "workspace extension is untrusted; retry with explicit workspace trust",
                    )?;
                Some(WorkspaceTrustRecord {
                    actor: actor.to_owned(),
                    approved_at: chrono::Utc::now().to_rfc3339(),
                    package_hash: hash.to_owned(),
                })
            }
            _ => None,
        };
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
            assets: result.manifest.assets.clone(),
            requested_permissions: result.manifest.requested_permissions.clone(),
            source,
            workspace_trust,
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
            "record": record,
        });
        self.store.store_receipt(pkg_id, &receipt)?;
        self.publish_evidence(
            "package_installed",
            pkg_id,
            Some(&record.version),
            "succeeded",
            vec![format!("package:sha256:{hash}")],
        )?;

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

    fn publish_evidence(
        &self,
        event_type: &str,
        package_id: &str,
        package_version: Option<&str>,
        result: &str,
        evidence_references: Vec<String>,
    ) -> Result<()> {
        self.store.append_evidence(&ExtensionEvidenceEvent {
            schema_version: 1,
            event_type: event_type.to_owned(),
            correlation_id: uuid::Uuid::new_v4().to_string(),
            package_id: package_id.to_owned(),
            package_version: package_version.map(str::to_owned),
            result: result.to_owned(),
            evidence_references,
            occurred_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

fn is_workspace_extension_path(path: &Path) -> bool {
    let mut saw_extensions = false;
    for component in path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .components()
        .rev()
    {
        let value = component.as_os_str();
        if !saw_extensions {
            saw_extensions = value == "extensions";
        } else if value == ".aletheon" {
            return true;
        } else {
            saw_extensions = value == "extensions";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{is_workspace_extension_path, ExtensionInstallService};
    use corpus::extension::store::{PackageSourceRecord, PackageStore};
    use flate2::{write::GzEncoder, Compression};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[test]
    fn recognizes_only_workspace_extension_directory() {
        assert!(is_workspace_extension_path(Path::new(
            "/tmp/project/.aletheon/extensions/pkg.tar.gz"
        )));
        assert!(!is_workspace_extension_path(Path::new(
            "/tmp/project/extensions/pkg.tar.gz"
        )));
    }

    fn create_package(root: &Path) -> PathBuf {
        let source = root.join("source");
        let skill = source.join("assets/skills/demo/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        let manifest = r#"
schema_version = 1
[package]
id = "test.workspace"
version = "1.0.0"
description = "workspace trust test"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "skill"
id = "skill.demo"
path = "assets/skills/demo/SKILL.md"
"#;
        let skill_body = "---\nname: demo\n---\n# Demo\n";
        std::fs::write(source.join("extension.toml"), manifest).unwrap();
        std::fs::write(&skill, skill_body).unwrap();
        let checksums = format!(
            "{:x}  extension.toml\n{:x}  assets/skills/demo/SKILL.md\n",
            Sha256::digest(manifest.as_bytes()),
            Sha256::digest(skill_body.as_bytes())
        );
        std::fs::write(source.join("checksums.sha256"), checksums).unwrap();

        let workspace = root.join("project/.aletheon/extensions");
        std::fs::create_dir_all(&workspace).unwrap();
        let archive = workspace.join("test.tar.gz");
        let encoder =
            GzEncoder::new(std::fs::File::create(&archive).unwrap(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder.append_dir_all(".", &source).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        archive
    }

    #[test]
    fn workspace_install_is_fail_closed_and_trust_is_hash_bound() {
        let temp = TempDir::new().unwrap();
        let archive = create_package(temp.path());
        let store_root = temp.path().join("store");
        let service = ExtensionInstallService::new(&store_root).unwrap();

        let error = service.install(&archive).unwrap_err().to_string();
        assert!(error.contains("workspace extension is untrusted"));
        assert!(service.list().unwrap().is_empty());

        let hash = service
            .install_with_workspace_trust(&archive, Some("operator:test"))
            .unwrap();
        let records = PackageStore::new(store_root)
            .unwrap()
            .get_installed("test.workspace")
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, PackageSourceRecord::Workspace);
        let trust = records[0].workspace_trust.as_ref().unwrap();
        assert_eq!(trust.actor, "operator:test");
        assert_eq!(trust.package_hash, hash);
    }
}
