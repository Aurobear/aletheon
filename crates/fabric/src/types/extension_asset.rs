//! Asset, Runtime, and Capability layer types for the Aletheon extension platform.
//!
//! - Asset: what a user installs (Skill, Hook, AgentProfile, Connector, Executable)
//! - Runtime: how an Asset executes (Native, Subprocess, Remote)
//! - Capability: what an Asset provides (Tool, HookProvider, AgentRuntimeProvider, ConnectorProvider)

use serde::{Deserialize, Serialize};

use super::admission::{CapabilityId, RiskLevel};
use super::extension_package::{PackageId, PermissionRequestSet};

// ---------------------------------------------------------------------------
// Asset identity and kind
// ---------------------------------------------------------------------------

/// What kind of logical asset this is. Deliberately excludes Tool, AgentRuntime,
/// and ProcessPlugin — those are Capability, Runtime, and isolation concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Skill,
    Hook,
    AgentProfile,
    Connector,
    Executable,
}

/// Fully-qualified asset identity: (package, name).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId {
    pub package: PackageId,
    pub name: String,
}

// ---------------------------------------------------------------------------
// Asset origin
// ---------------------------------------------------------------------------

/// Where an asset comes from. Distinct from ExtensionOrigin to separate
/// legacy and new models while allowing projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum AssetOrigin {
    BuiltIn,
    Package {
        package: String,
        version: String,
    },
    FileSystem {
        path: String,
    },
    Workspace {
        path: String,
    },
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// How an Asset executes. Does not contain product-specific commands or env vars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeClass {
    /// Built-in, compiled into the daemon binary with equal code review.
    Native,
    /// Isolated child process communicating via a standard protocol.
    Subprocess,
    /// External service reachable over the network.
    Remote,
}

/// Reference from an Asset to a required Runtime class and protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRef {
    pub class: RuntimeClass,
    /// Protocol identifier (e.g. "json-rpc/stdio", "grpc").
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
}

// ---------------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------------

/// Granular capability kind. Capability is the smallest unit of execution
/// and authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    HookProvider,
    AgentRuntimeProvider,
    ConnectorProvider,
}

/// Describes one capability an Asset declares it can provide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// Reuses the existing fabric::CapabilityId for identity continuity.
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    /// Reuses the existing fabric::RiskLevel for risk continuity.
    pub risk: RiskLevel,
}

// ---------------------------------------------------------------------------
// Asset descriptor
// ---------------------------------------------------------------------------

/// Immutable, read-only description of a single installable asset.
/// Separate from ExtensionDescriptor — this is the new model for Phase 2+.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDescriptor {
    pub id: AssetId,
    pub kind: AssetKind,
    pub version: String,
    pub description: String,
    pub origin: AssetOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_capabilities: Vec<CapabilityDescriptor>,
    #[serde(default)]
    pub requested_permissions: PermissionRequestSet,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg_id(s: &str) -> PackageId {
        PackageId(s.into())
    }

    #[test]
    fn asset_kind_serde_snake_case() {
        let cases = [
            (AssetKind::Skill, "skill"),
            (AssetKind::Hook, "hook"),
            (AssetKind::AgentProfile, "agent_profile"),
            (AssetKind::Connector, "connector"),
            (AssetKind::Executable, "executable"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let rt: AssetKind = serde_json::from_str(&json).unwrap();
            assert_eq!(rt, kind);
        }
    }

    #[test]
    fn asset_origin_tagged_serialization() {
        let builtin = AssetOrigin::BuiltIn;
        let json = serde_json::to_value(&builtin).unwrap();
        assert_eq!(json, serde_json::json!({"origin": "built_in"}));

        let pkg = AssetOrigin::Package {
            package: "pkg".into(),
            version: "1.0".into(),
        };
        let json = serde_json::to_value(&pkg).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"origin": "package", "package": "pkg", "version": "1.0"})
        );

        let ws = AssetOrigin::Workspace {
            path: "/repo".into(),
        };
        let json = serde_json::to_value(&ws).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"origin": "workspace", "path": "/repo"})
        );
    }

    #[test]
    fn runtime_class_serde() {
        let cases = [
            (RuntimeClass::Native, "native"),
            (RuntimeClass::Subprocess, "subprocess"),
            (RuntimeClass::Remote, "remote"),
        ];
        for (class, expected) in cases {
            let json = serde_json::to_string(&class).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
        }
    }

    #[test]
    fn capability_kind_serde() {
        let cases = [
            (CapabilityKind::Tool, "tool"),
            (CapabilityKind::HookProvider, "hook_provider"),
            (CapabilityKind::AgentRuntimeProvider, "agent_runtime_provider"),
            (CapabilityKind::ConnectorProvider, "connector_provider"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
        }
    }

    #[test]
    fn asset_descriptor_round_trip() {
        let desc = AssetDescriptor {
            id: AssetId {
                package: pkg_id("test.pkg"),
                name: "demo-skill".into(),
            },
            kind: AssetKind::Skill,
            version: "0.1.0".into(),
            description: "A demo skill".into(),
            origin: AssetOrigin::BuiltIn,
            runtime: None,
            declared_capabilities: vec![CapabilityDescriptor {
                id: CapabilityId("demo".into()),
                kind: CapabilityKind::Tool,
                risk: RiskLevel::ReadOnly,
            }],
            requested_permissions: PermissionRequestSet::default(),
        };
        let json = serde_json::to_string(&desc).unwrap();
        let rt: AssetDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.id.package, desc.id.package);
        assert_eq!(rt.id.name, desc.id.name);
        assert_eq!(rt.kind, AssetKind::Skill);
        assert_eq!(rt.declared_capabilities.len(), 1);
        assert_eq!(rt.declared_capabilities[0].kind, CapabilityKind::Tool);
    }

    #[test]
    fn asset_descriptor_with_runtime_round_trip() {
        let desc = AssetDescriptor {
            id: AssetId {
                package: pkg_id("test.pkg"),
                name: "runner".into(),
            },
            kind: AssetKind::Executable,
            version: "1.0.0".into(),
            description: "An executable asset".into(),
            origin: AssetOrigin::Package {
                package: "test.pkg".into(),
                version: "1.0.0".into(),
            },
            runtime: Some(RuntimeRef {
                class: RuntimeClass::Subprocess,
                protocol: "json-rpc/stdio".into(),
                min_version: Some("0.1.0".into()),
            }),
            declared_capabilities: vec![],
            requested_permissions: PermissionRequestSet {
                network: true,
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&desc).unwrap();
        let rt: AssetDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.runtime.as_ref().unwrap().class, RuntimeClass::Subprocess);
        assert!(rt.requested_permissions.network);
    }
}
