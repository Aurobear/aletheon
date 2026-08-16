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
use tokio::sync::RwLock;

use crate::extensions::extension_coordinator::ExtensionRuntimePublisher;
use crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot;

use super::extension_connectors::{connector_tool_owner, ExtensionConnectorRuntime};

pub struct DaemonExtensionRuntimePublisher {
    tools: Arc<Mutex<ToolRegistry>>,
    hooks: Arc<Mutex<HookRegistry>>,
    skills: SharedSkills,
    connectors: Arc<ExtensionConnectorRuntime>,
    profiles: RwLock<Option<PackageProfileRuntime>>,
    executables: RwLock<Option<Arc<super::extensions::ExtensionExecutableRuntime>>>,
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
            profiles: RwLock::new(None),
            executables: RwLock::new(None),
        }
    }

    pub async fn bind_profiles(&self, runtime: PackageProfileRuntime) -> Result<()> {
        let mut profiles = self.profiles.write().await;
        anyhow::ensure!(
            profiles.is_none(),
            "package Profile runtime is already bound"
        );
        *profiles = Some(runtime);
        Ok(())
    }

    async fn bind_executables(
        &self,
        runtime: Arc<super::extensions::ExtensionExecutableRuntime>,
    ) -> Result<()> {
        let mut executables = self.executables.write().await;
        anyhow::ensure!(
            executables.is_none(),
            "package executable runtime is already bound"
        );
        *executables = Some(runtime);
        Ok(())
    }

    pub async fn bind_executable_runtime(
        &self,
        data_root: &Path,
        store_root: &Path,
        clock: Arc<dyn ::contracts::Clock>,
    ) -> Result<Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter>> {
        let runtime = Arc::new(
            super::extensions::ExtensionExecutableRuntime::new(data_root, store_root, clock).await,
        );
        let router = runtime.router();
        self.bind_executables(runtime).await?;
        Ok(router)
    }

    pub async fn bind_runtime_agent_supervisor(
        &self,
        supervisor: Arc<runtime::RuntimeAgentSupervisor>,
        backend: Arc<dyn runtime::DelegateBackend>,
        agent_host: std::sync::Weak<
            dyn crate::wiring::application::agent_control::AgentHostEffects,
        >,
    ) -> Result<()> {
        let executables = self.executables.read().await.clone();
        if let Some(runtime) = executables {
            runtime
                .bind_runtime_supervisor(supervisor, backend, agent_host)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        Ok(())
    }
}

pub struct PackageProfileRuntime {
    registry: Arc<crate::wiring::adapters::runtime::AgentProfileRegistry>,
    inference: Arc<dyn cognit::ports::inference::InferencePort>,
    default_llm: Arc<dyn ::contracts::LlmProvider>,
    config: crate::config::CognitiveRuntimeConfig,
}

impl PackageProfileRuntime {
    pub fn new(
        registry: Arc<crate::wiring::adapters::runtime::AgentProfileRegistry>,
        inference: Arc<dyn cognit::ports::inference::InferencePort>,
        default_llm: Arc<dyn ::contracts::LlmProvider>,
        config: crate::config::CognitiveRuntimeConfig,
    ) -> Self {
        Self {
            registry,
            inference,
            default_llm,
            config,
        }
    }

    async fn prepare(
        &self,
        snapshot: &ExtensionRuntimeSnapshot,
        definitions: &[::contracts::ToolDefinition],
        profile_definitions: &[::contracts::ToolDefinition],
    ) -> Result<
        Vec<(
            String,
            crate::wiring::adapters::runtime::ResolvedAgentProfile,
        )>,
    > {
        let mut paths = BTreeMap::<String, Vec<PathBuf>>::new();
        for profile in snapshot.agent_profiles.iter() {
            paths
                .entry(profile.package_id.clone())
                .or_default()
                .push(profile.path.clone());
        }
        let mut prepared = Vec::new();
        for (owner, paths) in paths {
            let result = super::runtime::load_agent_profiles_from_paths(
                &paths,
                self.inference.clone(),
                self.default_llm.clone(),
                definitions,
                profile_definitions,
                &self.config,
                &crate::config::AgentProfilesConfig::default(),
            )
            .await?;
            anyhow::ensure!(
                result.quarantined.is_empty(),
                "package '{}' Agent Profile failed validation: {}",
                owner,
                result
                    .quarantined
                    .iter()
                    .map(|profile| profile.reason.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            prepared.extend(
                result
                    .registry
                    .resolved_profiles()
                    .into_iter()
                    .map(|profile| (owner.clone(), profile)),
            );
        }
        Ok(prepared)
    }

    fn publish(
        &self,
        owners: &[String],
        prepared: Vec<(
            String,
            crate::wiring::adapters::runtime::ResolvedAgentProfile,
        )>,
    ) -> Result<()> {
        self.registry
            .replace_package_profiles(owners, prepared)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
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
        if let Some(profiles) = self.profiles.read().await.as_ref() {
            let (mut definitions, mut profile_definitions) = self
                .tools
                .lock()
                .await
                .candidate_package_definitions(&prepared.tool_owners, &prepared.tool_sets)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let controls = corpus::tools::tools::agent_control::AgentControlTools::definitions();
            definitions.extend(controls.clone());
            profile_definitions.extend(controls);
            profiles
                .prepare(candidate, &definitions, &profile_definitions)
                .await?;
        }
        if let Some(executables) = self.executables.read().await.as_ref() {
            executables.probe(candidate).await?;
        }
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

        let prepared_profiles = if let Some(profiles) = self.profiles.read().await.as_ref() {
            let (mut definitions, mut profile_definitions) = self
                .tools
                .lock()
                .await
                .candidate_package_definitions(&tool_owners, &prepared.tool_sets)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let controls = corpus::tools::tools::agent_control::AgentControlTools::definitions();
            definitions.extend(controls.clone());
            profile_definitions.extend(controls);
            Some(
                profiles
                    .prepare(&candidate, &definitions, &profile_definitions)
                    .await?,
            )
        } else {
            None
        };

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
        if let (Some(profiles), Some(prepared_profiles)) =
            (self.profiles.read().await.as_ref(), prepared_profiles)
        {
            profiles.publish(
                &previous_packages
                    .union(&candidate_packages)
                    .cloned()
                    .collect::<Vec<_>>(),
                prepared_profiles,
            )?;
        }
        if let Some(executables) = self.executables.read().await.as_ref() {
            executables.publish(&previous, &candidate).await?;
        }
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
