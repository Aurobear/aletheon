//! Exact deployment verification and rollback planning.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentInfo {
    pub installed_sha: String,
    pub running_sha: String,
    pub binary_version: String,
    pub config_hash: String,
    pub binary_matches_installed: Option<bool>,
    pub runtime_versions_compatible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_core_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_user_runtime_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_plan: Option<RollbackPlan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub version_warnings: Vec<String>,
}

pub const CORE_RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Trusted record written by the deployment mechanism after installing both
/// runtime layers. Doctor reads this record; it never infers success from the
/// currently executing binary alone.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentManifest {
    pub installed_sha: String,
    pub core_runtime_version: String,
    pub user_runtime_version: String,
    pub previous_core_binary: PathBuf,
    pub previous_user_binary: PathBuf,
    pub previous_core_config: PathBuf,
    pub previous_user_config: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_binary: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_binary: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_config: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RollbackPlan {
    pub reason: String,
    pub restore_core_binary: PathBuf,
    pub restore_user_binary: PathBuf,
    pub restore_core_config: PathBuf,
    pub restore_user_config: PathBuf,
    pub target_core_binary: PathBuf,
    pub target_user_binary: PathBuf,
    pub target_core_config: PathBuf,
    pub target_user_config: PathBuf,
}

/// External deployment tooling may implement this port. Executive only plans
/// rollback and must not claim that files were restored without a real owner.
pub trait RollbackExecutor: Send + Sync {
    fn execute(&self, plan: &RollbackPlan) -> Result<()>;
}

/// Bounded filesystem rollback owner. Every source is validated and staged in
/// the destination directory before the first installed artifact is replaced.
pub struct FileRollbackExecutor {
    pub max_artifact_bytes: u64,
}

/// Receipt returned only after all four installed artifacts were restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentRollbackReceipt {
    pub installed_sha: String,
    pub restored_artifacts: usize,
}

/// Production owner for an explicitly requested deployment rollback. The
/// caller must bind the request to the manifest SHA it inspected, preventing
/// a stale diagnostic from rolling back a newer deployment.
#[derive(Clone)]
pub struct DeploymentRollbackService {
    manifest_path: PathBuf,
    executor: Arc<dyn RollbackExecutor>,
}

impl DeploymentRollbackService {
    pub fn filesystem(manifest_path: PathBuf) -> Self {
        Self {
            manifest_path,
            executor: Arc::new(FileRollbackExecutor::default()),
        }
    }

    pub fn execute_recommended(
        &self,
        expected_installed_sha: &str,
    ) -> Result<DeploymentRollbackReceipt> {
        anyhow::ensure!(
            !expected_installed_sha.is_empty(),
            "expected installed SHA is required"
        );
        let manifest = DeploymentManifest::load(&self.manifest_path)?;
        anyhow::ensure!(
            manifest.installed_sha == expected_installed_sha,
            "deployment manifest changed since rollback was confirmed"
        );
        let mut deployment = DeploymentInfo::gather();
        deployment.verify_manifest(&manifest);
        let plan = deployment
            .rollback_plan
            .ok_or_else(|| anyhow::anyhow!("deployment has no recommended rollback"))?;
        self.executor.execute(&plan)?;
        Ok(DeploymentRollbackReceipt {
            installed_sha: manifest.installed_sha,
            restored_artifacts: 4,
        })
    }
}

static ROLLBACK_FILE_NONCE: AtomicU64 = AtomicU64::new(0);

fn reserved_rollback_path(parent: &Path, index: usize, suffix: &str) -> Result<PathBuf> {
    for _ in 0..128 {
        let nonce = ROLLBACK_FILE_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".aletheon-rollback-{}-{nonce}-{index}.{suffix}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("reserve rollback artifact path"),
        }
    }
    anyhow::bail!("could not reserve a unique rollback artifact path")
}

impl Default for FileRollbackExecutor {
    fn default() -> Self {
        Self {
            max_artifact_bytes: 512 * 1024 * 1024,
        }
    }
}

impl RollbackExecutor for FileRollbackExecutor {
    fn execute(&self, plan: &RollbackPlan) -> Result<()> {
        let artifacts = [
            (&plan.restore_core_binary, &plan.target_core_binary),
            (&plan.restore_user_binary, &plan.target_user_binary),
            (&plan.restore_core_config, &plan.target_core_config),
            (&plan.restore_user_config, &plan.target_user_config),
        ];
        let mut staged = Vec::new();
        for (index, (source, target)) in artifacts.iter().enumerate() {
            let metadata = std::fs::symlink_metadata(source)
                .with_context(|| format!("inspect rollback source {}", source.display()))?;
            anyhow::ensure!(metadata.is_file(), "rollback source is not a regular file");
            anyhow::ensure!(
                metadata.len() <= self.max_artifact_bytes,
                "rollback artifact exceeds configured byte bound"
            );
            let parent = target
                .parent()
                .ok_or_else(|| anyhow::anyhow!("rollback target has no parent"))?;
            anyhow::ensure!(parent.is_dir(), "rollback target parent is unavailable");
            let stage = reserved_rollback_path(parent, index, "stage")?;
            if let Err(error) = std::fs::copy(source, &stage) {
                for (staged, _) in &staged {
                    let _ = std::fs::remove_file(staged);
                }
                return Err(error)
                    .with_context(|| format!("stage rollback artifact {}", source.display()));
            }
            staged.push((stage, (*target).clone()));
        }

        let mut replaced: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        for (index, (stage, target)) in staged.iter().enumerate() {
            let backup = if target.exists() {
                let parent = target.parent().expect("target parent was validated");
                let backup = reserved_rollback_path(parent, index, "current")?;
                std::fs::remove_file(&backup).context("release rollback backup reservation")?;
                Some(backup)
            } else {
                None
            };
            let replace = (|| -> Result<()> {
                if let Some(backup) = &backup {
                    std::fs::rename(target, backup)?;
                }
                std::fs::rename(stage, target)?;
                Ok(())
            })();
            if let Err(error) = replace {
                if let Some(backup) = &backup {
                    if !target.exists() {
                        let _ = std::fs::rename(backup, target);
                    }
                }
                for (restored_target, current_backup) in replaced.into_iter().rev() {
                    let _ = std::fs::remove_file(&restored_target);
                    if let Some(current_backup) = current_backup {
                        let _ = std::fs::rename(current_backup, restored_target);
                    }
                }
                for (stage, _) in &staged {
                    let _ = std::fs::remove_file(stage);
                }
                return Err(error).context("commit rollback artifacts");
            }
            replaced.push((target.clone(), backup));
        }
        for (_, backup) in replaced {
            if let Some(backup) = backup {
                let _ = std::fs::remove_file(backup);
            }
        }
        Ok(())
    }
}

impl DeploymentManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read deployment manifest {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("decode deployment manifest {}", path.display()))
    }

    fn rollback_plan(&self, reason: String) -> Result<RollbackPlan> {
        let required_target = |value: &Option<PathBuf>, name: &str| -> Result<PathBuf> {
            let value = value
                .clone()
                .ok_or_else(|| anyhow::anyhow!("deployment manifest lacks {name}"))?;
            anyhow::ensure!(
                !value.as_os_str().is_empty(),
                "deployment manifest has empty {name}"
            );
            Ok(value)
        };
        Ok(RollbackPlan {
            reason,
            restore_core_binary: self.previous_core_binary.clone(),
            restore_user_binary: self.previous_user_binary.clone(),
            restore_core_config: self.previous_core_config.clone(),
            restore_user_config: self.previous_user_config.clone(),
            target_core_binary: required_target(&self.core_binary, "core_binary")?,
            target_user_binary: required_target(&self.user_binary, "user_binary")?,
            target_core_config: required_target(&self.core_config, "core_config")?,
            target_user_config: required_target(&self.user_config, "user_config")?,
        })
    }
}

impl DeploymentInfo {
    pub fn gather() -> Self {
        Self {
            installed_sha: "unknown".into(),
            running_sha: option_env!("GIT_COMMIT_SHA").unwrap_or("unknown").into(),
            binary_version: env!("CARGO_PKG_VERSION").into(),
            config_hash: option_env!("CONFIG_HASH").unwrap_or("unknown").into(),
            binary_matches_installed: None,
            runtime_versions_compatible: None,
            installed_core_version: None,
            installed_user_runtime_version: None,
            rollback_plan: None,
            version_warnings: Vec::new(),
        }
    }

    pub fn verify_manifest(&mut self, manifest: &DeploymentManifest) {
        self.installed_sha = manifest.installed_sha.clone();
        let sha_matches = self.running_sha != "unknown"
            && !manifest.installed_sha.is_empty()
            && self.running_sha == manifest.installed_sha;
        self.binary_matches_installed = Some(sha_matches);
        if !sha_matches {
            self.version_warnings.push(format!(
                "installed SHA mismatch: running={}, manifest={}",
                self.running_sha, manifest.installed_sha
            ));
        }

        self.installed_core_version = Some(manifest.core_runtime_version.clone());
        self.installed_user_runtime_version = Some(manifest.user_runtime_version.clone());
        let versions_match = manifest.core_runtime_version == CORE_RUNTIME_VERSION
            && manifest.user_runtime_version == self.binary_version
            && manifest.core_runtime_version == manifest.user_runtime_version;
        self.runtime_versions_compatible = Some(versions_match);
        if !versions_match {
            self.version_warnings.push(format!(
                "runtime version mismatch: expected={}, core={}, user={}",
                self.binary_version, manifest.core_runtime_version, manifest.user_runtime_version
            ));
        }

        if !sha_matches || !versions_match {
            match manifest
                .rollback_plan("deployment SHA or core/user runtime version mismatch".into())
            {
                Ok(plan) => self.rollback_plan = Some(plan),
                Err(error) => self
                    .version_warnings
                    .push(format!("rollback unavailable: {error}")),
            }
        }
    }

    pub fn mark_manifest_unavailable(&mut self, error: impl std::fmt::Display) {
        self.binary_matches_installed = Some(false);
        self.runtime_versions_compatible = Some(false);
        self.version_warnings
            .push(format!("deployment manifest unavailable: {error}"));
    }

    pub fn is_healthy(&self) -> bool {
        self.binary_matches_installed == Some(true)
            && self.runtime_versions_compatible == Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(sha: &str, core: &str, user: &str) -> DeploymentManifest {
        DeploymentManifest {
            installed_sha: sha.into(),
            core_runtime_version: core.into(),
            user_runtime_version: user.into(),
            previous_core_binary: "/rollback/core".into(),
            previous_user_binary: "/rollback/user".into(),
            previous_core_config: "/rollback/core.toml".into(),
            previous_user_config: "/rollback/user.toml".into(),
            core_binary: Some("/installed/core".into()),
            user_binary: Some("/installed/user".into()),
            core_config: Some("/installed/core.toml".into()),
            user_config: Some("/installed/user.toml".into()),
        }
    }

    #[test]
    fn exact_sha_and_both_runtime_versions_are_required() {
        let mut info = DeploymentInfo::gather();
        info.running_sha = "abc".into();
        info.verify_manifest(&manifest(
            "abc",
            CORE_RUNTIME_VERSION,
            env!("CARGO_PKG_VERSION"),
        ));
        assert!(info.is_healthy());

        let mut mismatch = info.clone();
        mismatch.version_warnings.clear();
        mismatch.verify_manifest(&manifest("abcd", CORE_RUNTIME_VERSION, "0.0.0"));
        assert!(!mismatch.is_healthy());
        let plan = mismatch.rollback_plan.unwrap();
        assert_eq!(plan.restore_core_binary, PathBuf::from("/rollback/core"));
        assert_eq!(
            plan.restore_user_config,
            PathBuf::from("/rollback/user.toml")
        );
    }

    #[test]
    fn filesystem_rollback_restores_both_binaries_and_configs() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup");
        let installed = root.path().join("installed");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        let names = ["core", "user", "core.toml", "user.toml"];
        for name in names {
            std::fs::write(backup.join(name), format!("previous-{name}")).unwrap();
            std::fs::write(installed.join(name), format!("current-{name}")).unwrap();
        }
        let plan = RollbackPlan {
            reason: "version mismatch".into(),
            restore_core_binary: backup.join("core"),
            restore_user_binary: backup.join("user"),
            restore_core_config: backup.join("core.toml"),
            restore_user_config: backup.join("user.toml"),
            target_core_binary: installed.join("core"),
            target_user_binary: installed.join("user"),
            target_core_config: installed.join("core.toml"),
            target_user_config: installed.join("user.toml"),
        };
        FileRollbackExecutor::default().execute(&plan).unwrap();
        for name in names {
            assert_eq!(
                std::fs::read_to_string(installed.join(name)).unwrap(),
                format!("previous-{name}")
            );
        }
    }

    #[test]
    fn production_rollback_service_binds_confirmation_sha_and_restores_all_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup");
        let installed = root.path().join("installed");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        for name in ["core", "user", "core.toml", "user.toml"] {
            std::fs::write(backup.join(name), format!("previous-{name}")).unwrap();
            std::fs::write(installed.join(name), format!("current-{name}")).unwrap();
        }
        let manifest_path = root.path().join("deployment-manifest.json");
        let manifest = DeploymentManifest {
            installed_sha: "installed-sha".into(),
            core_runtime_version: "mismatched-core".into(),
            user_runtime_version: "mismatched-user".into(),
            previous_core_binary: backup.join("core"),
            previous_user_binary: backup.join("user"),
            previous_core_config: backup.join("core.toml"),
            previous_user_config: backup.join("user.toml"),
            core_binary: Some(installed.join("core")),
            user_binary: Some(installed.join("user")),
            core_config: Some(installed.join("core.toml")),
            user_config: Some(installed.join("user.toml")),
        };
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let service = DeploymentRollbackService::filesystem(manifest_path);

        assert!(service.execute_recommended("stale-sha").is_err());
        for name in ["core", "user", "core.toml", "user.toml"] {
            assert_eq!(
                std::fs::read_to_string(installed.join(name)).unwrap(),
                format!("current-{name}")
            );
        }

        let receipt = service.execute_recommended("installed-sha").unwrap();
        assert_eq!(receipt.installed_sha, "installed-sha");
        assert_eq!(receipt.restored_artifacts, 4);
        for name in ["core", "user", "core.toml", "user.toml"] {
            assert_eq!(
                std::fs::read_to_string(installed.join(name)).unwrap(),
                format!("previous-{name}")
            );
        }
    }

    #[test]
    fn rollback_validation_failure_changes_no_installed_artifact() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed");
        std::fs::create_dir_all(&installed).unwrap();
        for name in ["core", "user", "core.toml", "user.toml"] {
            std::fs::write(installed.join(name), format!("current-{name}")).unwrap();
        }
        let missing = root.path().join("missing");
        let plan = RollbackPlan {
            reason: "test".into(),
            restore_core_binary: missing.clone(),
            restore_user_binary: missing.clone(),
            restore_core_config: missing.clone(),
            restore_user_config: missing,
            target_core_binary: installed.join("core"),
            target_user_binary: installed.join("user"),
            target_core_config: installed.join("core.toml"),
            target_user_config: installed.join("user.toml"),
        };
        assert!(FileRollbackExecutor::default().execute(&plan).is_err());
        for name in ["core", "user", "core.toml", "user.toml"] {
            assert_eq!(
                std::fs::read_to_string(installed.join(name)).unwrap(),
                format!("current-{name}")
            );
        }
    }

    #[test]
    fn legacy_manifest_deserializes_without_unsafe_rollback_targets() {
        let legacy = serde_json::json!({
            "installed_sha": "old",
            "core_runtime_version": "0.0.0",
            "user_runtime_version": "0.0.0",
            "previous_core_binary": "/rollback/core",
            "previous_user_binary": "/rollback/user",
            "previous_core_config": "/rollback/core.toml",
            "previous_user_config": "/rollback/user.toml"
        });
        let manifest: DeploymentManifest = serde_json::from_value(legacy).unwrap();
        assert!(manifest.core_binary.is_none());

        let mut info = DeploymentInfo::gather();
        info.running_sha = "new".into();
        info.verify_manifest(&manifest);
        assert!(info.rollback_plan.is_none());
        assert!(info
            .version_warnings
            .iter()
            .any(|warning| warning.contains("rollback unavailable")));
    }

    #[test]
    fn concurrent_rollbacks_use_distinct_artifact_paths() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup");
        let installed = root.path().join("installed");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        for name in ["core", "user", "core.toml", "user.toml"] {
            std::fs::write(backup.join(name), format!("previous-{name}")).unwrap();
        }

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let barrier = barrier.clone();
                let backup = &backup;
                let installed = &installed;
                scope.spawn(move || {
                    let worker_dir = installed.join(worker.to_string());
                    std::fs::create_dir_all(&worker_dir).unwrap();
                    for name in ["core", "user", "core.toml", "user.toml"] {
                        std::fs::write(worker_dir.join(name), "current").unwrap();
                    }
                    let plan = RollbackPlan {
                        reason: "concurrency test".into(),
                        restore_core_binary: backup.join("core"),
                        restore_user_binary: backup.join("user"),
                        restore_core_config: backup.join("core.toml"),
                        restore_user_config: backup.join("user.toml"),
                        target_core_binary: worker_dir.join("core"),
                        target_user_binary: worker_dir.join("user"),
                        target_core_config: worker_dir.join("core.toml"),
                        target_user_config: worker_dir.join("user.toml"),
                    };
                    barrier.wait();
                    FileRollbackExecutor::default().execute(&plan).unwrap();
                });
            }
        });
        for worker in 0..8 {
            for name in ["core", "user", "core.toml", "user.toml"] {
                assert_eq!(
                    std::fs::read_to_string(installed.join(worker.to_string()).join(name)).unwrap(),
                    format!("previous-{name}")
                );
            }
        }
    }
}
