//! Immutable compilation boundary for enabled extension package assets.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use corpus::extension::resolver::{ResolvedPackageAsset, ResolvedPackageSet};
use corpus::extension::store::ActivationRecord;
use fabric::protocol::extension::McpConnectorManifestV1;
use fabric::types::extension_asset::AssetKind;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ExtensionConnectorAsset {
    pub package_id: String,
    pub package_root: PathBuf,
    pub manifest: McpConnectorManifestV1,
}

#[derive(Debug, Clone)]
pub struct ExtensionAgentProfileAsset {
    pub package_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ExtensionRuntimeSnapshot {
    pub digest: String,
    pub skills: Arc<Vec<corpus::skill::loader::LoadedSkill>>,
    pub skill_plugins: Arc<Vec<corpus::skill::plugin::SkillPlugin>>,
    pub hooks: Arc<Vec<corpus::hook::loader::HookConfig>>,
    pub connectors: Arc<Vec<ExtensionConnectorAsset>>,
    pub agent_profiles: Arc<Vec<ExtensionAgentProfileAsset>>,
    pub executable_assets: Arc<Vec<ResolvedPackageAsset>>,
    pub package_digests: Arc<BTreeMap<String, String>>,
    pub activation_records: Arc<BTreeMap<String, ActivationRecord>>,
}

impl ExtensionRuntimeSnapshot {
    pub fn empty() -> Self {
        Self {
            digest: digest_json(&serde_json::json!({
                "packages": {},
                "assets": [],
                "configured_mcp_ids": []
            })),
            skills: Arc::new(Vec::new()),
            skill_plugins: Arc::new(Vec::new()),
            hooks: Arc::new(Vec::new()),
            connectors: Arc::new(Vec::new()),
            agent_profiles: Arc::new(Vec::new()),
            executable_assets: Arc::new(Vec::new()),
            package_digests: Arc::new(BTreeMap::new()),
            activation_records: Arc::new(BTreeMap::new()),
        }
    }
}

#[derive(Clone)]
pub struct ExtensionRuntimeView {
    current: Arc<RwLock<Arc<ExtensionRuntimeSnapshot>>>,
}

impl ExtensionRuntimeView {
    pub fn new(initial: ExtensionRuntimeSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    pub async fn load(&self) -> Arc<ExtensionRuntimeSnapshot> {
        self.current.read().await.clone()
    }

    pub async fn publish(&self, snapshot: ExtensionRuntimeSnapshot) {
        *self.current.write().await = Arc::new(snapshot);
    }
}

impl Default for ExtensionRuntimeView {
    fn default() -> Self {
        Self::new(ExtensionRuntimeSnapshot::empty())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionSnapshotCompiler {
    configured_mcp_ids: BTreeSet<String>,
}

impl ExtensionSnapshotCompiler {
    pub fn new(configured_mcp_ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            configured_mcp_ids: configured_mcp_ids.into_iter().collect(),
        }
    }

    pub fn compile(&self, resolved: &ResolvedPackageSet) -> Result<ExtensionRuntimeSnapshot> {
        let mut assets = resolved.assets.clone();
        assets.sort_by(|left, right| {
            kind_name(left.asset.kind)
                .cmp(kind_name(right.asset.kind))
                .then(left.asset.id.cmp(&right.asset.id))
                .then(left.package_id.cmp(&right.package_id))
                .then(left.package_hash.cmp(&right.package_hash))
        });

        let mut package_digests = BTreeMap::new();
        let mut skills = Vec::new();
        let mut skill_plugins = Vec::new();
        let mut hooks = Vec::new();
        let mut connectors = Vec::new();
        let mut agent_profiles = Vec::new();
        let mut executable_assets = Vec::new();
        let mut public_skill_names = BTreeSet::new();
        let mut connector_ids = BTreeSet::new();
        let mut canonical_assets = Vec::new();

        for resolved_asset in assets {
            match package_digests.entry(resolved_asset.package_id.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(resolved_asset.package_hash.clone());
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    anyhow::ensure!(
                        entry.get() == &resolved_asset.package_hash,
                        "package '{}' resolves to multiple active hashes",
                        resolved_asset.package_id
                    );
                }
            }

            let package_root = package_root_for(&resolved_asset)?;
            match resolved_asset.asset.kind {
                AssetKind::Skill => {
                    let skill_dir = resolved_asset
                        .absolute_path
                        .parent()
                        .context("Skill asset path has no parent directory")?;
                    let (mut loaded, plugin) = corpus::skill::loader::load_skill_dir(skill_dir)?;
                    anyhow::ensure!(
                        public_skill_names.insert(loaded.name.clone()),
                        "duplicate public skill name '{}'",
                        loaded.name
                    );
                    loaded.source = format!(
                        "package:{}:{}",
                        resolved_asset.package_id, resolved_asset.asset.id
                    );
                    canonical_assets.push(serde_json::json!({
                        "kind": "skill",
                        "id": resolved_asset.asset.id,
                        "package": resolved_asset.package_id,
                        "hash": resolved_asset.package_hash,
                        "public_name": loaded.name,
                    }));
                    skills.push(loaded);
                    skill_plugins.push(plugin);
                }
                AssetKind::Hook => {
                    let mut hook = corpus::hook::loader::load_hook_path(
                        &resolved_asset.absolute_path,
                        Some(&package_root),
                    )?;
                    hook.name = format!("{}:{}", resolved_asset.package_id, hook.name);
                    canonical_assets.push(serde_json::json!({
                        "kind": "hook",
                        "id": resolved_asset.asset.id,
                        "package": resolved_asset.package_id,
                        "hash": resolved_asset.package_hash,
                        "runtime_name": hook.name,
                        "timeout_ms": hook.timeout_ms,
                    }));
                    hooks.push(hook);
                }
                AssetKind::Connector => {
                    let content = std::fs::read_to_string(&resolved_asset.absolute_path)?;
                    let connector: McpConnectorManifestV1 =
                        serde_json::from_str(&content).context("parsing MCP connector manifest")?;
                    connector.validate()?;
                    anyhow::ensure!(
                        !self.configured_mcp_ids.contains(&connector.id),
                        "package connector '{}' conflicts with administrator configuration",
                        connector.id
                    );
                    anyhow::ensure!(
                        connector_ids.insert(connector.id.clone()),
                        "duplicate package connector id '{}'",
                        connector.id
                    );
                    canonical_assets.push(serde_json::json!({
                        "kind": "connector",
                        "id": resolved_asset.asset.id,
                        "package": resolved_asset.package_id,
                        "hash": resolved_asset.package_hash,
                        "connector": connector,
                    }));
                    connectors.push(ExtensionConnectorAsset {
                        package_id: resolved_asset.package_id.clone(),
                        package_root,
                        manifest: connector,
                    });
                }
                AssetKind::AgentProfile => {
                    let content = std::fs::read_to_string(&resolved_asset.absolute_path)?;
                    anyhow::ensure!(!content.trim().is_empty(), "Agent Profile asset is empty");
                    canonical_assets.push(serde_json::json!({
                        "kind": "agent_profile",
                        "id": resolved_asset.asset.id,
                        "package": resolved_asset.package_id,
                        "hash": resolved_asset.package_hash,
                    }));
                    agent_profiles.push(ExtensionAgentProfileAsset {
                        package_id: resolved_asset.package_id.clone(),
                        path: resolved_asset.absolute_path.clone(),
                    });
                }
                AssetKind::Executable => {
                    let content = std::fs::read_to_string(&resolved_asset.absolute_path)?;
                    corpus::extension::manifest::parse_executable_runtime_manifest(&content)
                        .context("parsing executable runtime manifest")?;
                    canonical_assets.push(serde_json::json!({
                        "kind": "executable",
                        "id": resolved_asset.asset.id,
                        "package": resolved_asset.package_id,
                        "hash": resolved_asset.package_hash,
                    }));
                    executable_assets.push(resolved_asset);
                }
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name));
        skill_plugins.sort_by(|left, right| left.name.cmp(&right.name));
        hooks.sort_by(|left, right| left.name.cmp(&right.name));
        connectors.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        agent_profiles.sort_by(|left, right| {
            left.package_id
                .cmp(&right.package_id)
                .then(left.path.cmp(&right.path))
        });
        executable_assets.sort_by(|left, right| left.asset.id.cmp(&right.asset.id));
        let activation_records = resolved
            .activation_records
            .iter()
            .map(|record| (record.package_id.clone(), record.clone()))
            .collect::<BTreeMap<_, _>>();
        let digest = digest_json(&serde_json::json!({
            "packages": package_digests,
            "assets": canonical_assets,
            "configured_mcp_ids": self.configured_mcp_ids,
        }));

        Ok(ExtensionRuntimeSnapshot {
            digest,
            skills: Arc::new(skills),
            skill_plugins: Arc::new(skill_plugins),
            hooks: Arc::new(hooks),
            connectors: Arc::new(connectors),
            agent_profiles: Arc::new(agent_profiles),
            executable_assets: Arc::new(executable_assets),
            package_digests: Arc::new(package_digests),
            activation_records: Arc::new(activation_records),
        })
    }
}

fn package_root_for(asset: &ResolvedPackageAsset) -> Result<PathBuf> {
    let mut root = asset.absolute_path.clone();
    for _ in Path::new(&asset.asset.path).components() {
        anyhow::ensure!(
            root.pop(),
            "asset path has more components than its package root"
        );
    }
    anyhow::ensure!(root.is_dir(), "resolved package root is not a directory");
    Ok(root)
}

fn kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::AgentProfile => "agent_profile",
        AssetKind::Connector => "connector",
        AssetKind::Executable => "executable",
        AssetKind::Hook => "hook",
        AssetKind::Skill => "skill",
    }
}

fn digest_json(value: &serde_json::Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
