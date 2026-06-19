use serde::{Deserialize, Serialize};

/// Plugin entry point type — replaces fragile string prefix parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryType {
    /// Shell script entry point.
    Cmd(String),
    /// Native shared library.
    Native(String),
    /// WebAssembly module.
    Wasm(String),
}

impl EntryType {
    /// Get the entry path.
    pub fn path(&self) -> &str {
        match self {
            Self::Cmd(p) | Self::Native(p) | Self::Wasm(p) => p,
        }
    }
}

impl std::str::FromStr for EntryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (prefix, path) = s.split_once(':').ok_or_else(|| {
            format!(
                "Entry '{}' missing type prefix (expected 'cmd:', 'native:', 'wasm:')",
                s
            )
        })?;
        match prefix {
            "cmd" => Ok(Self::Cmd(path.to_string())),
            "native" => Ok(Self::Native(path.to_string())),
            "wasm" => Ok(Self::Wasm(path.to_string())),
            other => Err(format!("Unknown entry type '{}'", other)),
        }
    }
}

/// Plugin manifest (plugin.toml).
///
/// Supports two formats:
/// - Flat (legacy): fields at top level
/// - Nested (new): `[plugin]` section with `entry` field using type prefix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Plugin version (semver).
    pub version: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Author.
    #[serde(default)]
    pub author: String,
    /// Entry point with type prefix (e.g. "cmd:./run.sh", "native:./lib.so").
    /// For legacy manifests without a prefix, treated as a raw path.
    #[serde(default)]
    pub entry: String,
    /// Tools provided by this plugin.
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    /// Hooks provided by this plugin.
    #[serde(default)]
    pub hooks: Vec<PluginHookDef>,
    /// Dependencies on other plugins.
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    /// Minimum agent version required.
    pub min_agent_version: Option<String>,
    /// Permissions required.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Optional structured permissions (filesystem, network).
    pub plugin_permissions: Option<PluginPermissions>,
}

/// Tool definition in plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub permission_level: String,
}

/// Hook definition in plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginHookDef {
    pub event: String,
    pub handler: String,
}

/// Plugin dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDependency {
    pub id: String,
    pub version_req: String,
    #[serde(default)]
    pub optional: bool,
}

/// Structured permissions for a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPermissions {
    pub filesystem: Option<Vec<String>>,
    pub network: Option<Vec<String>>,
}

impl PluginManifest {
    /// Load manifest from a TOML file.
    pub fn from_file(path: &std::path::Path) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Self = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// Validate the manifest.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("Plugin ID cannot be empty".into());
        }
        if self.version.is_empty() {
            return Err("Plugin version cannot be empty".into());
        }
        if self.entry.is_empty() {
            return Err("Plugin entry point cannot be empty".into());
        }
        self.parsed_entry()?; // validate entry format
        Ok(())
    }

    /// Parse the entry string into a typed EntryType.
    pub fn parsed_entry(&self) -> Result<EntryType, String> {
        self.entry.parse()
    }

    /// Get the entry type prefix (e.g. "cmd", "native", "wasm").
    pub fn entry_type(&self) -> &str {
        self.entry.splitn(2, ':').next().unwrap_or("")
    }

    /// Get the entry path portion (after the type prefix colon).
    pub fn entry_path(&self) -> &str {
        self.entry.splitn(2, ':').nth(1).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(entry: &str) -> PluginManifest {
        PluginManifest {
            id: "test-plugin".into(),
            name: "Test Plugin".into(),
            version: "0.1.0".into(),
            description: "A test plugin".into(),
            author: "test".into(),
            entry: entry.into(),
            tools: vec![],
            hooks: vec![],
            dependencies: vec![],
            min_agent_version: None,
            permissions: vec![],
            plugin_permissions: None,
        }
    }

    #[test]
    fn test_manifest_validation() {
        let manifest = make_manifest("cmd:./run.sh");
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_empty_id_validation() {
        let mut manifest = make_manifest("cmd:./run.sh");
        manifest.id = "".into();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_empty_entry_validation() {
        let manifest = make_manifest("");
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_entry_type_parsing() {
        let manifest = make_manifest("cmd:./run.sh");
        assert_eq!(manifest.entry_type(), "cmd");
        assert_eq!(manifest.entry_path(), "./run.sh");

        let manifest = make_manifest("native:./lib.so");
        assert_eq!(manifest.entry_type(), "native");
        assert_eq!(manifest.entry_path(), "./lib.so");
    }

    #[test]
    fn test_entry_type_enum() {
        assert_eq!(
            "cmd:./run.sh".parse::<EntryType>().unwrap(),
            EntryType::Cmd("./run.sh".into())
        );
        assert_eq!(
            "native:./lib.so".parse::<EntryType>().unwrap(),
            EntryType::Native("./lib.so".into())
        );
        assert_eq!(
            "wasm:./module.wasm".parse::<EntryType>().unwrap(),
            EntryType::Wasm("./module.wasm".into())
        );
        assert!("bad_path".parse::<EntryType>().is_err());
        assert!("unknown:./x".parse::<EntryType>().is_err());
    }

    #[test]
    fn test_parsed_entry() {
        let manifest = make_manifest("cmd:./run.sh");
        assert_eq!(
            manifest.parsed_entry().unwrap(),
            EntryType::Cmd("./run.sh".into())
        );

        let bad = make_manifest("bad");
        assert!(bad.parsed_entry().is_err());
    }

    #[test]
    fn test_toml_parsing_nested() {
        let toml_str = r#"
id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "Test"
author = "test"
entry = "cmd:./run.sh"

[[tools]]
name = "search"
description = "Search tool"
input_schema = {}
permission_level = "L0"
"#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.id, "my-plugin");
        assert_eq!(manifest.entry, "cmd:./run.sh");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "search");
    }
}
