//! Exact deployment verification and rollback planning.

use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RollbackPlan {
    pub reason: String,
    pub restore_core_binary: PathBuf,
    pub restore_user_binary: PathBuf,
    pub restore_core_config: PathBuf,
    pub restore_user_config: PathBuf,
}

/// External deployment tooling may implement this port. Executive only plans
/// rollback and must not claim that files were restored without a real owner.
#[allow(dead_code)]
pub trait RollbackExecutor {
    fn execute(&self, plan: &RollbackPlan) -> Result<()>;
}

impl DeploymentManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read deployment manifest {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("decode deployment manifest {}", path.display()))
    }

    fn rollback_plan(&self, reason: String) -> RollbackPlan {
        RollbackPlan {
            reason,
            restore_core_binary: self.previous_core_binary.clone(),
            restore_user_binary: self.previous_user_binary.clone(),
            restore_core_config: self.previous_core_config.clone(),
            restore_user_config: self.previous_user_config.clone(),
        }
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
            self.rollback_plan = Some(
                manifest
                    .rollback_plan("deployment SHA or core/user runtime version mismatch".into()),
            );
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
}
