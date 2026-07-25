//! Stable extension metadata shared by discovery, policy, and protocol clients.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::admission::{CapabilityId, RiskLevel};
use crate::ToolDefinition;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtensionContractError {
    #[error("extension name must be non-empty and contain no controls or ':'")]
    InvalidName,
    #[error("extension version must be non-empty")]
    InvalidVersion,
    #[error("extension capability must be non-empty")]
    InvalidCapability,
    #[error("tool definition '{definition}' does not match capability '{capability}'")]
    ToolCapabilityMismatch {
        definition: String,
        capability: String,
    },
}

/// Stable deterministic identity: `<kind>:<local-name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(kind: ExtensionKind, local_name: &str) -> Result<Self, ExtensionContractError> {
        let local_name = local_name.trim();
        if local_name.is_empty()
            || local_name
                .chars()
                .any(|character| character.is_control() || character == ':')
        {
            return Err(ExtensionContractError::InvalidName);
        }
        Ok(Self(format!("{}:{local_name}", kind.as_str())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Tool,
    Skill,
    Hook,
    Plugin,
    Mcp,
}

impl ExtensionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
            Self::Hook => "hook",
            Self::Plugin => "plugin",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ExtensionOrigin {
    BuiltIn,
    FileSystem { path: String },
    Package { package: String },
    Remote { server: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationConstraints {
    #[serde(default)]
    pub allowed_agents: Vec<String>,
    #[serde(default)]
    pub required_config_flags: Vec<String>,
    #[serde(default)]
    pub requires_approval: bool,
}

/// Immutable discovery metadata. Discovery is intentionally separate from activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionDescriptor {
    pub id: ExtensionId,
    pub kind: ExtensionKind,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<CapabilityId>,
    pub origin: ExtensionOrigin,
    pub activation: ActivationConstraints,
    pub risk: RiskLevel,
    pub tool_definition: Option<ToolDefinition>,
}

impl ExtensionDescriptor {
    pub fn new(
        kind: ExtensionKind,
        local_name: impl AsRef<str>,
        version: impl Into<String>,
        description: impl Into<String>,
        capability: CapabilityId,
        risk: RiskLevel,
    ) -> Result<Self, ExtensionContractError> {
        if capability.0.trim().is_empty() {
            return Err(ExtensionContractError::InvalidCapability);
        }
        let version = version.into();
        if version.trim().is_empty() {
            return Err(ExtensionContractError::InvalidVersion);
        }
        Ok(Self {
            id: ExtensionId::new(kind, local_name.as_ref())?,
            kind,
            version,
            description: description.into(),
            capabilities: vec![capability],
            origin: ExtensionOrigin::BuiltIn,
            activation: ActivationConstraints::default(),
            risk,
            tool_definition: None,
        })
    }

    pub fn with_origin(mut self, origin: ExtensionOrigin) -> Self {
        self.origin = origin;
        self
    }
    pub fn with_activation_constraints(mut self, activation: ActivationConstraints) -> Self {
        self.activation = activation;
        self
    }

    pub fn with_tool_definition(
        mut self,
        definition: ToolDefinition,
    ) -> Result<Self, ExtensionContractError> {
        let capability = self
            .primary_capability()
            .map(|value| value.0.as_str())
            .unwrap_or_default();
        if definition.name != capability {
            return Err(ExtensionContractError::ToolCapabilityMismatch {
                definition: definition.name,
                capability: capability.to_string(),
            });
        }
        self.tool_definition = Some(definition);
        Ok(self)
    }

    pub fn primary_capability(&self) -> Option<&CapabilityId> {
        self.capabilities.first()
    }
    pub fn is_executable(&self) -> bool {
        matches!(self.kind, ExtensionKind::Tool | ExtensionKind::Mcp)
            && self.tool_definition.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionSnapshot {
    pub entries: Vec<ExtensionDescriptor>,
}

/// Read-only catalog contract used by policy and protocol layers.
pub trait ExtensionCatalog: Send + Sync {
    fn snapshot(&self) -> ExtensionSnapshot;
}

// ---------------------------------------------------------------------------
// Compatibility projections: old → new types
// ---------------------------------------------------------------------------
// These From impls allow existing code to keep using ExtensionDescriptor /
// ExtensionKind / ExtensionOrigin, while new code can project them into
// the Phase 1 layered model (AssetDescriptor, AssetKind, AssetOrigin).
// All projections are pure read-only conversions.

impl From<ExtensionKind> for super::extension_asset::AssetKind {
    fn from(kind: ExtensionKind) -> Self {
        match kind {
            ExtensionKind::Tool => super::extension_asset::AssetKind::Executable,
            ExtensionKind::Skill => super::extension_asset::AssetKind::Skill,
            ExtensionKind::Hook => super::extension_asset::AssetKind::Hook,
            ExtensionKind::Plugin => super::extension_asset::AssetKind::Executable,
            ExtensionKind::Mcp => super::extension_asset::AssetKind::Connector,
        }
    }
}

impl From<ExtensionOrigin> for super::extension_asset::AssetOrigin {
    fn from(origin: ExtensionOrigin) -> Self {
        match origin {
            ExtensionOrigin::BuiltIn => super::extension_asset::AssetOrigin::BuiltIn,
            ExtensionOrigin::FileSystem { path } => {
                super::extension_asset::AssetOrigin::FileSystem { path }
            }
            ExtensionOrigin::Package { package } => super::extension_asset::AssetOrigin::Package {
                package,
                version: String::new(),
            },
            ExtensionOrigin::Remote { server } => {
                // Remote origins map to a synthetic package identity
                super::extension_asset::AssetOrigin::Package {
                    package: format!("remote:{server}"),
                    version: String::new(),
                }
            }
        }
    }
}

impl From<&ExtensionDescriptor> for super::extension_asset::AssetDescriptor {
    fn from(desc: &ExtensionDescriptor) -> Self {
        use super::extension_package::PackageId;
        let (package_name, asset_name) = match desc.id.as_str().split_once(':') {
            Some((kind, name)) => (format!("legacy:{kind}"), name.to_string()),
            None => ("legacy:unknown".to_string(), desc.id.as_str().to_string()),
        };
        let capabilities: Vec<super::extension_asset::CapabilityDescriptor> = desc
            .capabilities
            .iter()
            .map(|cap| super::extension_asset::CapabilityDescriptor {
                id: cap.clone(),
                kind: super::extension_asset::CapabilityKind::Tool,
                risk: desc.risk,
            })
            .collect();
        super::extension_asset::AssetDescriptor {
            id: super::extension_asset::AssetId {
                package: PackageId(package_name),
                name: asset_name,
            },
            kind: desc.kind.into(),
            version: desc.version.clone(),
            description: desc.description.clone(),
            origin: desc.origin.clone().into(),
            runtime: None,
            declared_capabilities: capabilities,
            requested_permissions: super::extension_package::PermissionRequestSet::default(),
        }
    }
}
