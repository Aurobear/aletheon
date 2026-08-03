//! Live publication of package-owned Skill tools, catalogs, and Hooks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use corpus::hook::registry::{HookRegistry, RegisteredHook};
use corpus::tools::tools::script_tool::ScriptTool;
use corpus::tools::tools::skill_tools::SharedSkills;
use corpus::tools::tools::{Tool, ToolRegistry};
use tokio::sync::Mutex;

use crate::application::extension_coordinator::ExtensionRuntimePublisher;
use crate::application::extension_snapshot::ExtensionRuntimeSnapshot;

use super::extension_connectors::{connector_tool_owner, ExtensionConnectorRuntime};

pub struct DaemonExtensionRuntimePublisher {
    tools: Arc<Mutex<ToolRegistry>>,
    hooks: Arc<Mutex<HookRegistry>>,
    skills: SharedSkills,
    connectors: Arc<ExtensionConnectorRuntime>,
}

impl DaemonExtensionRuntimePublisher {
    pub fn new(
        tools: Arc<Mutex<ToolRegistry>>,
        hooks: Arc<Mutex<HookRegistry>>,
        skills: SharedSkills,
    ) -> Self {
        Self {
            tools,
            hooks,
            skills,
            connectors: Arc::new(ExtensionConnectorRuntime::default()),
        }
    }
}

#[async_trait]
impl ExtensionRuntimePublisher for DaemonExtensionRuntimePublisher {
    async fn probe(&self, candidate: &ExtensionRuntimeSnapshot) -> Result<()> {
        self.connectors.probe(candidate).await?;
        let mut prepared = PreparedRuntime::build(candidate)?;
        prepared
            .tool_sets
            .extend(self.connectors.tool_sets(candidate).await?);
        prepared.tool_owners = prepared
            .tool_sets
            .iter()
            .map(|(owner, _)| owner.clone())
            .collect();
        self.skills.validate_extensions(candidate.skills.as_ref())?;
        self.tools
            .lock()
            .await
            .validate_package_tool_sets(&prepared.tool_owners, &prepared.tool_sets)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(())
    }

    async fn publish(
        &self,
        previous: Arc<ExtensionRuntimeSnapshot>,
        candidate: Arc<ExtensionRuntimeSnapshot>,
    ) -> Result<()> {
        self.connectors.probe(&candidate).await?;
        let mut prepared = PreparedRuntime::build(&candidate)?;
        prepared
            .tool_sets
            .extend(self.connectors.tool_sets(&candidate).await?);
        self.skills.validate_extensions(candidate.skills.as_ref())?;

        let previous_packages: BTreeSet<_> = previous.package_digests.keys().cloned().collect();
        let candidate_packages: BTreeSet<_> = candidate.package_digests.keys().cloned().collect();
        let hook_owners: BTreeSet<_> = previous_packages
            .union(&candidate_packages)
            .cloned()
            .collect();
        let mut tool_owners: BTreeSet<_> = hook_owners
            .iter()
            .map(|owner| skill_tool_owner(owner))
            .collect();
        for connector in previous
            .connectors
            .iter()
            .chain(candidate.connectors.iter())
        {
            tool_owners.insert(connector_tool_owner(&connector.package_id));
        }
        tool_owners.extend(prepared.tool_sets.iter().map(|(owner, _)| owner.clone()));
        let tool_owners: Vec<_> = tool_owners.into_iter().collect();

        let mut tools = self.tools.lock().await;
        let mut hooks = self.hooks.lock().await;
        tools
            .replace_package_tool_sets(&tool_owners, prepared.tool_sets)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        for owner in hook_owners {
            hooks.replace_package_hooks(
                &owner,
                prepared.hooks.get(&owner).cloned().unwrap_or_default(),
            );
        }
        self.skills
            .replace_extensions(candidate.skills.as_ref().clone())?;
        self.connectors
            .publish(&previous.digest, &candidate.digest)
            .await;
        Ok(())
    }
}

struct PreparedRuntime {
    tool_owners: Vec<String>,
    tool_sets: Vec<(String, Vec<Arc<dyn Tool>>)>,
    hooks: BTreeMap<String, Vec<RegisteredHook>>,
}

impl PreparedRuntime {
    fn build(snapshot: &ExtensionRuntimeSnapshot) -> Result<Self> {
        let mut tool_sets = BTreeMap::<String, Vec<Arc<dyn Tool>>>::new();
        let mut hooks = BTreeMap::<String, Vec<RegisteredHook>>::new();
        let mut public_hook_names = BTreeSet::new();

        for plugin in snapshot.skill_plugins.iter() {
            let loaded = snapshot
                .skills
                .iter()
                .find(|skill| skill.name == plugin.name)
                .with_context(|| format!("Skill plugin '{}' has no catalog entry", plugin.name))?;
            let package_id = package_id_from_skill_source(&loaded.source)?;
            let tool_owner = skill_tool_owner(package_id);
            for definition in &plugin.tools {
                let script = contained_skill_script(
                    &plugin.skill_dir,
                    &plugin.scripts_dir,
                    &definition.script,
                )?;
                let mut tool = ScriptTool::new(
                    definition.name.clone(),
                    definition.description.clone(),
                    script,
                    definition.permission,
                )
                .with_exposure(definition.exposure);
                if let Some(schema) = &definition.input_schema {
                    tool = tool.with_schema(schema.clone());
                }
                tool_sets
                    .entry(tool_owner.clone())
                    .or_default()
                    .push(Arc::new(tool));
            }
            for definition in &plugin.hooks {
                let script = contained_skill_script(
                    &plugin.skill_dir,
                    &plugin.scripts_dir,
                    &definition.script,
                )?;
                let name = format!("{package_id}:{}:{}", plugin.name, definition.name);
                anyhow::ensure!(
                    public_hook_names.insert(name.clone()),
                    "duplicate package Hook name '{name}'"
                );
                hooks
                    .entry(package_id.to_owned())
                    .or_default()
                    .push(RegisteredHook {
                        name,
                        source: format!("package:{package_id}"),
                        script_path: Some(script),
                        point: definition.point,
                        priority: definition.priority,
                        timeout_ms: None,
                    });
            }
        }

        for hook in snapshot.hooks.iter() {
            let package_id = package_id_from_hook_name(&hook.name, snapshot)?;
            anyhow::ensure!(
                public_hook_names.insert(hook.name.clone()),
                "duplicate package Hook name '{}'",
                hook.name
            );
            hooks
                .entry(package_id.to_owned())
                .or_default()
                .push(RegisteredHook {
                    name: hook.name.clone(),
                    source: format!("package:{package_id}"),
                    script_path: Some(hook.script.clone()),
                    point: hook.point,
                    priority: hook.priority,
                    timeout_ms: hook.timeout_ms,
                });
        }

        for entries in hooks.values_mut() {
            entries.sort_by(|left, right| {
                left.priority
                    .cmp(&right.priority)
                    .then(left.name.cmp(&right.name))
            });
        }
        let tool_owners = tool_sets.keys().cloned().collect();
        Ok(Self {
            tool_owners,
            tool_sets: tool_sets.into_iter().collect(),
            hooks,
        })
    }
}

fn skill_tool_owner(package_id: &str) -> String {
    format!("extension-skill:{package_id}")
}

fn package_id_from_skill_source(source: &str) -> Result<&str> {
    let package_asset = source
        .strip_prefix("package:")
        .context("package Skill has no package source")?;
    let (package_id, _) = package_asset
        .rsplit_once(':')
        .context("package Skill source has no asset ID")?;
    anyhow::ensure!(
        !package_id.is_empty(),
        "package Skill source has empty package ID"
    );
    Ok(package_id)
}

fn package_id_from_hook_name<'a>(
    name: &str,
    snapshot: &'a ExtensionRuntimeSnapshot,
) -> Result<&'a str> {
    snapshot
        .package_digests
        .keys()
        .filter(|package_id| name.starts_with(&format!("{package_id}:")))
        .max_by_key(|package_id| package_id.len())
        .map(String::as_str)
        .with_context(|| format!("package Hook '{name}' has no snapshot owner"))
}

fn contained_skill_script(
    skill_root: &Path,
    scripts_dir: &Path,
    declared: &str,
) -> Result<PathBuf> {
    let relative = Path::new(declared);
    anyhow::ensure!(
        !declared.trim().is_empty()
            && !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "Skill script must be a contained relative path"
    );
    let root = skill_root
        .canonicalize()
        .context("canonicalizing package Skill root")?;
    let script = scripts_dir
        .join(relative)
        .canonicalize()
        .with_context(|| format!("package Skill script does not exist: {declared}"))?;
    anyhow::ensure!(
        script.starts_with(root),
        "package Skill script escapes its root"
    );
    anyhow::ensure!(script.is_file(), "package Skill script is not a file");
    Ok(script)
}
