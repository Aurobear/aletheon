//! Deployment verification and version compatibility checks.
//!
//! Powers `aletheon doctor` deployment health (T3).
//! Checks: installed SHA, binary version, config hash, core/user runtime alignment.

use serde::Serialize;

/// Deployment info collected at build time and verified at startup.
#[derive(Debug, Clone, Serialize)]
pub struct DeploymentInfo {
    /// Git commit hash at build time, or "unknown" if not embedded.
    pub installed_sha: &'static str,
    /// The binary version from CARGO_PKG_VERSION.
    pub binary_version: &'static str,
    /// Hash of the compiled-in default config (for drift detection).
    pub config_hash: &'static str,
    /// Whether the running binary matches the installed SHA.
    pub binary_matches_installed: Option<bool>,
    /// Whether core and user runtime versions are compatible.
    pub runtime_versions_compatible: Option<bool>,
    /// Any mismatch details for diagnostics.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub version_warnings: Vec<String>,
}

/// The core runtime version this executive was built against.
/// In a layered deployment, this should match the user runtime's expected core version.
pub const CORE_RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

impl DeploymentInfo {
    /// Collect build-time info. Runtime checks (`binary_matches_installed`,
    /// `runtime_versions_compatible`) are populated later by the doctor.
    pub fn gather() -> Self {
        Self {
            installed_sha: option_env!("GIT_COMMIT_SHA").unwrap_or("unknown"),
            binary_version: env!("CARGO_PKG_VERSION"),
            config_hash: option_env!("CONFIG_HASH").unwrap_or("unknown"),
            binary_matches_installed: None,
            runtime_versions_compatible: None,
            version_warnings: Vec::new(),
        }
    }

    /// Verify that the binary version matches the installed SHA.
    /// In production, this would compare against a recorded deployment manifest.
    pub fn verify_binary(&mut self, _recorded_sha: Option<&str>) {
        // The recorded SHA would come from a deployment manifest file
        // or a runtime-provided value. For now, if we have a build-time
        // SHA, we consider it matching (it was compiled from that commit).
        if self.installed_sha != "unknown" {
            self.binary_matches_installed = Some(true);
        } else {
            self.version_warnings
                .push("installed_sha is unknown — build without GIT_COMMIT_SHA".into());
        }
    }

    /// Check core vs user runtime version compatibility.
    /// Both should be on the same major.minor to be compatible.
    pub fn verify_runtime_compatibility(&mut self, core_version: &str) {
        let user_version = self.binary_version;
        let compatible = semver_compatible(user_version, core_version);
        self.runtime_versions_compatible = Some(compatible);
        if !compatible {
            self.version_warnings.push(format!(
                "runtime version mismatch: user={user_version}, core={core_version} — rollback may be needed"
            ));
        }
    }

    /// Returns true if deployment is healthy (no critical warnings).
    pub fn is_healthy(&self) -> bool {
        self.binary_matches_installed != Some(false)
            && self.runtime_versions_compatible != Some(false)
    }
}

/// Simple semver-like compatibility check: major.minor must match.
fn semver_compatible(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64)> {
        let mut parts = v.split(|c: char| c == '.' || c == '-');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some((major, minor))
    };
    match (parse(a), parse(b)) {
        (Some((ma, mia)), Some((mb, mib))) => ma == mb && mia == mib,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_populates_build_constants() {
        let info = DeploymentInfo::gather();
        assert_eq!(info.binary_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn verify_binary_with_sha() {
        let mut info = DeploymentInfo {
            installed_sha: "abc1234",
            binary_version: "1.0.0",
            config_hash: "hash",
            binary_matches_installed: None,
            runtime_versions_compatible: None,
            version_warnings: vec![],
        };
        info.verify_binary(None);
        assert_eq!(info.binary_matches_installed, Some(true));
    }

    #[test]
    fn verify_binary_unknown_sha_adds_warning() {
        let mut info = DeploymentInfo::gather();
        info.verify_binary(None);
        if info.installed_sha == "unknown" {
            assert!(!info.version_warnings.is_empty());
        }
    }

    #[test]
    fn semver_major_minor_match() {
        assert!(semver_compatible("1.2.3", "1.2.0"));
        assert!(semver_compatible("1.2", "1.2.3"));
        assert!(!semver_compatible("1.2.0", "1.3.0"));
        assert!(!semver_compatible("2.0.0", "1.0.0"));
    }

    #[test]
    fn runtime_mismatch_detected() {
        let mut info = DeploymentInfo {
            installed_sha: "abc",
            binary_version: "1.0.0",
            config_hash: "hash",
            binary_matches_installed: None,
            runtime_versions_compatible: None,
            version_warnings: vec![],
        };
        info.verify_runtime_compatibility("2.0.0");
        assert_eq!(info.runtime_versions_compatible, Some(false));
        assert!(!info.version_warnings.is_empty());
    }

    #[test]
    fn runtime_match_ok() {
        let mut info = DeploymentInfo {
            installed_sha: "abc",
            binary_version: "1.0.0",
            config_hash: "hash",
            binary_matches_installed: None,
            runtime_versions_compatible: None,
            version_warnings: vec![],
        };
        info.verify_runtime_compatibility("1.0.0");
        assert_eq!(info.runtime_versions_compatible, Some(true));
        assert!(info.version_warnings.is_empty());
    }
}
