//! Package-layer types for the Aletheon extension platform.
//!
//! Package is the distribution and version-management container.
//! It can contain multiple Assets but does not execute itself.

use serde::{Deserialize, Serialize};

// Re-use AssetKind from the asset module. When extension_asset.rs is also
// in the same crate, we can reference it directly.
use super::extension_asset::AssetKind;

// ---------------------------------------------------------------------------
// Package identity
// ---------------------------------------------------------------------------

/// Publisher-namespaced package identifier (e.g. "aletheon.core-tools").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageId(pub String);

/// Semver-like version string (e.g. "1.2.3").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageVersion(pub String);

// ---------------------------------------------------------------------------
// Compatibility
// ---------------------------------------------------------------------------

/// Minimum / maximum Aletheon version range this package supports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilitySpec {
    /// Minimum supported Aletheon version (inclusive), e.g. "0.1.0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_aletheon: Option<String>,
    /// Maximum supported Aletheon version (inclusive), e.g. "1.0.0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_aletheon: Option<String>,
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// Coarse permission summary declared at the package level.
/// Runtime grants must NOT exceed what is declared here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequestSet {
    /// Allowed filesystem paths (empty = none, absent = none declared).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<Vec<String>>,
    /// Whether network access is requested.
    #[serde(default)]
    pub network: bool,
    /// Whether the package contains executable assets that spawn processes.
    #[serde(default)]
    pub executables: bool,
}

// ---------------------------------------------------------------------------
// Asset reference (index entry in PackageManifest)
// ---------------------------------------------------------------------------

/// Lightweight index entry — one per asset declared in extension.toml.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    pub kind: AssetKind,
    /// Asset-local identifier (e.g. "skill.demo").
    pub id: String,
    /// Package-relative path to the asset's manifest file.
    pub path: String,
}

// ---------------------------------------------------------------------------
// Package info (nested for TOML [package] table compatibility)
// ---------------------------------------------------------------------------

/// Package identity and metadata — corresponds to TOML `[package]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    pub id: PackageId,
    pub version: PackageVersion,
    pub description: String,
    #[serde(default)]
    pub compatibility: CompatibilitySpec,
}

// ---------------------------------------------------------------------------
// Package manifest
// ---------------------------------------------------------------------------

/// Top-level manifest for an extension package (extension.toml).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    /// Schema version for forward compatibility. Must be 1 for v1 format.
    pub schema_version: u16,
    pub package: PackageInfo,
    /// Index of assets declared in this package.
    pub assets: Vec<AssetRef>,
    #[serde(default)]
    pub requested_permissions: PermissionRequestSet,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_id_serde_transparent() {
        let id = PackageId("test.minimal".into());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"test.minimal\"");
        let rt: PackageId = serde_json::from_str(&json).unwrap();
        assert_eq!(rt, id);
    }

    #[test]
    fn package_manifest_round_trip() {
        let manifest = PackageManifest {
            schema_version: 1,
            package: PackageInfo {
                id: PackageId("test.minimal".into()),
                version: PackageVersion("0.1.0".into()),
                description: "Minimal legal extension package".into(),
                compatibility: CompatibilitySpec {
                    min_aletheon: Some("0.1.0".into()),
                    max_aletheon: None,
                },
            },
            assets: vec![AssetRef {
                kind: AssetKind::Skill,
                id: "skill.demo".into(),
                path: "assets/skills/demo/SKILL.md".into(),
            }],
            requested_permissions: PermissionRequestSet::default(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let rt: PackageManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.package.id, manifest.package.id);
        assert_eq!(rt.schema_version, 1);
        assert_eq!(rt.assets.len(), 1);
        assert_eq!(rt.assets[0].kind, AssetKind::Skill);
    }

    #[test]
    fn permission_request_set_defaults() {
        let json = r#"{}"#;
        let prs: PermissionRequestSet = serde_json::from_str(json).unwrap();
        assert!(!prs.network);
        assert!(!prs.executables);
        assert!(prs.filesystem.is_none());
    }
}
