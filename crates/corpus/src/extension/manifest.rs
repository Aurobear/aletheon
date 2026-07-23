//! Manifest parsing for extension packages.
//!
//! Parses extension.toml into fabric::PackageManifest and validates
//! structure. Asset manifests (SKILL.md, hook.toml, etc.) are parsed
//! lazily during the inspector phase.

use anyhow::{bail, Context, Result};
use fabric::{CapabilityDescriptor, CapabilityKind, PackageManifest, RuntimeClass};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIsolationManifest {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub filesystem: Vec<String>,
    pub cpu_time_seconds: u64,
    pub memory_bytes: u64,
    pub max_processes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutableRuntimeManifest {
    pub schema_version: u16,
    pub id: String,
    pub class: RuntimeClass,
    pub protocol: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub secret_refs: BTreeMap<String, String>,
    pub isolation: RuntimeIsolationManifest,
    pub capabilities: Vec<CapabilityDescriptor>,
}

pub fn parse_executable_runtime_manifest(content: &str) -> Result<ExecutableRuntimeManifest> {
    let manifest: ExecutableRuntimeManifest =
        toml::from_str(content).context("failed to parse executable runtime manifest")?;
    anyhow::ensure!(manifest.schema_version == 1, "unsupported runtime schema version");
    anyhow::ensure!(manifest.class == RuntimeClass::Subprocess, "third-party runtime must use subprocess class");
    anyhow::ensure!(manifest.protocol == "json-rpc/stdio", "unsupported runtime protocol");
    anyhow::ensure!(!manifest.id.trim().is_empty(), "runtime ID must not be empty");
    super::validation::validate_entry_path(Path::new(&manifest.command))?;
    anyhow::ensure!(
        manifest.command.starts_with("payload/"),
        "runtime command must be inside payload/"
    );
    anyhow::ensure!(
        manifest.isolation.cpu_time_seconds > 0
            && manifest.isolation.memory_bytes > 0
            && manifest.isolation.max_processes > 0,
        "runtime resource limits must be nonzero"
    );
    anyhow::ensure!(!manifest.capabilities.is_empty(), "runtime must declare capabilities");
    anyhow::ensure!(
        manifest
            .capabilities
            .iter()
            .any(|capability| capability.kind == CapabilityKind::AgentRuntimeProvider),
        "runtime manifest does not provide an Agent Runtime capability"
    );
    for (environment, secret_ref) in &manifest.secret_refs {
        anyhow::ensure!(
            !environment.trim().is_empty() && !secret_ref.trim().is_empty(),
            "secret references require nonempty environment and reference names"
        );
        anyhow::ensure!(
            !secret_ref.contains('=') && !secret_ref.chars().any(char::is_whitespace),
            "secret reference contains forbidden characters"
        );
    }
    Ok(manifest)
}

/// Parse an extension.toml file into a PackageManifest.
pub fn parse_package_manifest(content: &str) -> Result<PackageManifest> {
    let manifest: PackageManifest = toml::from_str(content)
        .context("failed to parse extension.toml")?;
    validate_package_manifest(&manifest)?;
    Ok(manifest)
}

/// Validate a PackageManifest after parsing.
pub fn validate_package_manifest(manifest: &PackageManifest) -> Result<()> {
    if manifest.schema_version != 1 {
        bail!(
            "unsupported schema_version {} (expected 1)",
            manifest.schema_version
        );
    }
    if manifest.package.id.0.trim().is_empty() {
        bail!("package id must not be empty");
    }
    if manifest.package.version.0.trim().is_empty() {
        bail!("package version must not be empty");
    }
    if manifest.package.description.trim().is_empty() {
        bail!("package description must not be empty");
    }
    if manifest.assets.is_empty() {
        bail!("package must declare at least one asset");
    }

    // Check for duplicate asset IDs
    let mut seen = std::collections::HashSet::new();
    for asset in &manifest.assets {
        if asset.id.trim().is_empty() {
            bail!("asset id must not be empty");
        }
        if asset.path.trim().is_empty() {
            bail!("asset path must not be empty (asset: {})", asset.id);
        }
        if !seen.insert(&asset.id) {
            bail!("duplicate asset id: {}", asset.id);
        }
    }

    // Validate publisher namespace
    if manifest.package.id.0.starts_with("aletheon.") && !manifest.package.id.0.starts_with("aletheon.builtin.") {
        bail!(
            "reserved namespace 'aletheon.*' may not be used by third-party packages"
        );
    }

    Ok(())
}

/// Parse checksums.sha256 file. Returns HashMap<filename, sha256_hex>.
pub fn parse_checksums(content: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut checksums = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |c: char| c.is_whitespace());
        let hash = parts.next().unwrap_or("").trim().to_lowercase();
        let path = parts.next().unwrap_or("").trim();
        if hash.is_empty()
            || path.is_empty()
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("invalid checksum line: {}", line);
        }
        if checksums.contains_key(path) {
            bail!("duplicate checksum path: {}", path);
        }
        checksums.insert(path.to_string(), hash.to_string());
    }
    Ok(checksums)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_manifest() {
        let toml = r#"
schema_version = 1

[package]
id = "test.minimal"
version = "0.1.0"
description = "A test package"

[[assets]]
kind = "skill"
id = "skill.demo"
path = "assets/skills/demo/SKILL.md"
"#;
        let manifest = parse_package_manifest(toml).unwrap();
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.package.id.0, "test.minimal");
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(manifest.assets[0].id, "skill.demo");
    }

    #[test]
    fn reject_empty_id() {
        let toml = r#"
schema_version = 1

[package]
id = ""
version = "0.1.0"
description = "test"
"#;
        assert!(parse_package_manifest(toml).is_err());
    }

    #[test]
    fn reject_no_assets() {
        let toml = r#"
schema_version = 1

[package]
id = "test.pkg"
version = "0.1.0"
description = "test"
"#;
        assert!(parse_package_manifest(toml).is_err());
    }

    #[test]
    fn reject_reserved_namespace() {
        let toml = r#"
schema_version = 1

[package]
id = "aletheon.private"
version = "0.1.0"
description = "test"

[[assets]]
kind = "skill"
id = "s"
path = "s.md"
"#;
        let err = parse_package_manifest(toml).unwrap_err().to_string();
        assert!(err.contains("reserved namespace"));
    }

    #[test]
    fn reject_duplicate_assets() {
        let toml = r#"
schema_version = 1

[package]
id = "test.pkg"
version = "0.1.0"
description = "test"

[[assets]]
kind = "skill"
id = "same-id"
path = "a.md"

[[assets]]
kind = "hook"
id = "same-id"
path = "b.toml"
"#;
        let err = parse_package_manifest(toml).unwrap_err().to_string();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn parse_checksums_valid() {
        let content = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  extension.toml\n";
        let map = parse_checksums(content).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("extension.toml"));
    }

    #[test]
    fn parse_checksums_short_hash_rejected() {
        assert!(parse_checksums("abc  file.txt").is_err());
    }

    #[test]
    fn executable_runtime_manifest_is_generic_and_resource_bounded() {
        let manifest = parse_executable_runtime_manifest(
            r#"
schema_version = 1
id = "runtime.generic"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "payload/runtime"
args = ["--stdio"]

[isolation]
network = false
filesystem = []
cpu_time_seconds = 30
memory_bytes = 268435456
max_processes = 8

[[capabilities]]
id = "agent.generic"
kind = "agent_runtime_provider"
risk = "Sandboxed"
"#,
        )
        .unwrap();
        assert_eq!(manifest.id, "runtime.generic");
        assert!(!manifest.isolation.network);
    }

    #[test]
    fn executable_runtime_manifest_rejects_unsafe_or_unbounded_configuration() {
        let base = r#"
schema_version = 1
id = "runtime.generic"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "../escape"
[isolation]
network = false
cpu_time_seconds = 0
memory_bytes = 0
max_processes = 0
[[capabilities]]
id = "agent.generic"
kind = "agent_runtime_provider"
risk = "Sandboxed"
"#;
        assert!(parse_executable_runtime_manifest(base).is_err());
        assert!(parse_executable_runtime_manifest(&base.replace(
            "class = \"subprocess\"",
            "class = \"native\""
        ))
        .is_err());
        assert!(parse_executable_runtime_manifest(&format!("{base}\nunknown = true\n")).is_err());
    }
}
