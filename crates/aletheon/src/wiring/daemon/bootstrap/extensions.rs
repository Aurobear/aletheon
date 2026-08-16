//! Scoped composition of non-tool Corpus extensions.

use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot;
use tokio::sync::RwLock;

use super::extension_provider_launcher::ExtensionProviderLauncher;

pub(super) struct ExtensionExecutableRuntime {
    data_root: std::path::PathBuf,
    store_root: std::path::PathBuf,
    sandbox: Option<Arc<dyn ::contracts::SandboxBackend>>,
    router: Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter>,
    staged_runtimes: Arc<PackageRuntimeStaging>,
    runtime_agent_supervisor: RwLock<Option<Arc<runtime::RuntimeAgentSupervisor>>>,
    runtime_backend: RwLock<Option<Arc<dyn runtime::DelegateBackend>>>,
    agent_host: RwLock<
        Option<std::sync::Weak<dyn crate::wiring::application::agent_control::AgentHostEffects>>,
    >,
    managed_runtime_ids: std::sync::Mutex<HashSet<runtime::DelegateBackendId>>,
}

/// Package-owned launchers staged before the Runtime supervisor is bound.
/// This is not a selection authority: entries are only materialized into the
/// supervisor's DelegateBackendRegistry at bind/reload time.
#[derive(Default)]
struct PackageRuntimeStaging {
    launchers: std::sync::RwLock<
        HashMap<
            ::contracts::RuntimeId,
            Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher>,
        >,
    >,
    manifests: std::sync::RwLock<HashMap<::contracts::RuntimeId, runtime::RuntimeManifest>>,
    owners: std::sync::RwLock<HashMap<::contracts::RuntimeId, String>>,
}

impl PackageRuntimeStaging {
    fn runtime_ids(&self) -> Vec<::contracts::RuntimeId> {
        let mut ids = self
            .launchers
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    fn catalog(&self) -> Vec<runtime::RuntimeManifest> {
        let mut manifests = self
            .manifests
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.id.cmp(&right.id));
        manifests
    }

    fn resolve(
        &self,
        id: &::contracts::RuntimeId,
    ) -> Option<Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher>> {
        self.launchers.read().unwrap().get(id).cloned()
    }

    fn validate_package_runtimes(
        &self,
        replaced_owners: &[String],
        replacements: &[(
            String,
            ::contracts::RuntimeId,
            Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher>,
        )],
    ) -> anyhow::Result<()> {
        let replaced = replaced_owners.iter().collect::<HashSet<_>>();
        let launchers = self.launchers.read().unwrap();
        let owners = self.owners.read().unwrap();
        let mut ids = HashSet::new();
        for (owner, id, _) in replacements {
            anyhow::ensure!(
                !id.0.trim().is_empty() && replaced.contains(owner),
                "invalid package runtime replacement"
            );
            anyhow::ensure!(ids.insert(id), "duplicate package runtime: {}", id.0);
            if launchers.contains_key(id)
                && owners
                    .get(id)
                    .is_none_or(|existing| !replaced.contains(existing))
            {
                anyhow::bail!("runtime already registered: {}", id.0);
            }
        }
        Ok(())
    }

    fn replace_package_runtimes(
        &self,
        replaced_owners: &[String],
        replacements: Vec<(
            String,
            ::contracts::RuntimeId,
            Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher>,
        )>,
    ) -> anyhow::Result<()> {
        self.validate_package_runtimes(replaced_owners, &replacements)?;
        let replaced = replaced_owners.iter().collect::<HashSet<_>>();
        let mut launchers = self.launchers.write().unwrap();
        let mut owners = self.owners.write().unwrap();
        let removed = owners
            .iter()
            .filter(|(_, owner)| replaced.contains(owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in removed {
            owners.remove(&id);
            launchers.remove(&id);
            self.manifests.write().unwrap().remove(&id);
        }
        for (owner, id, launcher) in replacements {
            owners.insert(id.clone(), owner);
            launchers.insert(id, launcher);
        }
        Ok(())
    }
}

impl ExtensionExecutableRuntime {
    pub async fn new(
        data_root: &Path,
        store_root: &Path,
        clock: Arc<dyn ::contracts::Clock>,
    ) -> Self {
        let sandbox = corpus::security::sandbox::BubblewrapBackend::probe_async(clock)
            .await
            .map(|backend| Arc::new(backend) as Arc<dyn ::contracts::SandboxBackend>);
        Self {
            data_root: data_root.to_owned(),
            store_root: store_root.to_owned(),
            sandbox,
            router: Arc::new(
                crate::extensions::extension_runtime_router::ExtensionRuntimeRouter::default(),
            ),
            staged_runtimes: Arc::new(PackageRuntimeStaging::default()),
            runtime_agent_supervisor: RwLock::new(None),
            runtime_backend: RwLock::new(None),
            agent_host: RwLock::new(None),
            managed_runtime_ids: std::sync::Mutex::new(HashSet::new()),
        }
    }

    pub fn router(
        &self,
    ) -> Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter> {
        self.router.clone()
    }

    /// Bind the Runtime AgentSupervisor after daemon AgentControl has been
    /// composed. Package reloads then update the same Runtime catalog used by
    /// admission; running bindings remain pinned by the supervisor.
    pub async fn bind_runtime_supervisor(
        &self,
        supervisor: Arc<runtime::RuntimeAgentSupervisor>,
        backend: Arc<dyn runtime::DelegateBackend>,
        agent_host: std::sync::Weak<dyn crate::wiring::application::agent_control::AgentHostEffects>,
    ) -> anyhow::Result<()> {
        {
            let mut slot = self.runtime_agent_supervisor.write().await;
            anyhow::ensure!(slot.is_none(), "Runtime AgentSupervisor is already bound");
            *slot = Some(supervisor);
        }
        *self.runtime_backend.write().await = Some(backend);
        *self.agent_host.write().await = Some(agent_host);
        self.sync_runtime_catalog().await
    }

    async fn sync_runtime_catalog(&self) -> anyhow::Result<()> {
        let supervisor = self.runtime_agent_supervisor.read().await.clone();
        let backend = self.runtime_backend.read().await.clone();
        let Some(supervisor) = supervisor else {
            return Ok(());
        };
        let Some(backend) = backend else {
            return Ok(());
        };
        let agent_host = self
            .agent_host
            .read()
            .await
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        let desired = self.staged_runtimes.runtime_ids();
        let manifests = self.staged_runtimes.catalog();
        let desired_ids = desired
            .iter()
            .map(|id| runtime::DelegateBackendId(id.0.clone()))
            .collect::<HashSet<_>>();
        let managed_ids = self.managed_runtime_ids.lock().unwrap().clone();
        for id in &desired_ids {
            if supervisor.registry().resolve(id).is_some() {
                if managed_ids.contains(id) {
                    if let Some(manifest) = manifests.iter().find(|manifest| manifest.id == id.0) {
                        supervisor
                            .replace_backend_manifest(id.clone(), manifest.clone())
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                }
                continue;
            }
            if let Some(manifest) = manifests.iter().find(|manifest| manifest.id == id.0) {
                let runtime_backend = agent_host
                    .as_ref()
                    .and_then(|host| {
                        self.staged_runtimes
                            .resolve(&::contracts::RuntimeId(id.0.clone()))
                            .map(|launcher| {
                                Arc::new(
                                    crate::wiring::application::agent_control::RuntimeObservedAgentBackend::pinned(
                                        host,
                                        launcher,
                                    ),
                                ) as Arc<dyn runtime::DelegateBackend>
                            })
                    })
                    .unwrap_or_else(|| backend.clone());
                supervisor
                    .register_backend_with_manifest(id.clone(), runtime_backend, manifest.clone())
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            } else {
                let runtime_backend = agent_host
                    .as_ref()
                    .and_then(|host| {
                        self.staged_runtimes
                            .resolve(&::contracts::RuntimeId(id.0.clone()))
                            .map(|launcher| {
                                Arc::new(
                                    crate::wiring::application::agent_control::RuntimeObservedAgentBackend::pinned(
                                        host,
                                        launcher,
                                    ),
                                ) as Arc<dyn runtime::DelegateBackend>
                            })
                    })
                    .unwrap_or_else(|| backend.clone());
                supervisor
                    .register_backend(id.clone(), runtime_backend)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            }
            self.managed_runtime_ids.lock().unwrap().insert(id.clone());
        }
        let stale = self
            .managed_runtime_ids
            .lock()
            .unwrap()
            .iter()
            .filter(|id| !desired_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in stale {
            supervisor.unregister_backend(&id);
            self.managed_runtime_ids.lock().unwrap().remove(&id);
        }
        Ok(())
    }

    pub async fn probe(&self, snapshot: &ExtensionRuntimeSnapshot) -> anyhow::Result<()> {
        self.prepare(snapshot).await.map(|_| ())
    }

    pub async fn publish(
        &self,
        previous: &ExtensionRuntimeSnapshot,
        candidate: &ExtensionRuntimeSnapshot,
    ) -> anyhow::Result<()> {
        let prepared = self.prepare(candidate).await?;
        let owners = previous
            .package_digests
            .keys()
            .chain(candidate.package_digests.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let providers = prepared
            .iter()
            .map(|entry| {
                (
                    entry.owner.clone(),
                    entry.id.clone(),
                    entry.provider.clone(),
                )
            })
            .collect::<Vec<_>>();
        self.router
            .validate_package_providers(&owners, &providers)?;
        let launcher: Arc<dyn crate::wiring::application::agent_control::AgentRuntimeLauncher> =
            Arc::new(ExtensionProviderLauncher::new(self.router.clone()));
        let launchers = prepared
            .iter()
            .map(|entry| (entry.owner.clone(), entry.id.clone(), launcher.clone()))
            .collect::<Vec<_>>();
        self.staged_runtimes
            .validate_package_runtimes(&owners, &launchers)?;
        self.router.replace_package_providers(&owners, providers)?;
        self.staged_runtimes
            .replace_package_runtimes(&owners, launchers)?;
        self.sync_runtime_catalog().await?;
        Ok(())
    }

    async fn prepare(
        &self,
        snapshot: &ExtensionRuntimeSnapshot,
    ) -> anyhow::Result<Vec<PreparedExecutable>> {
        use ::contracts::{ResolvedSandboxPolicy, RuntimeId, SandboxConfig, WorkspacePolicy};
        use corpus::extension::manifest::parse_executable_runtime_manifest;

        if snapshot.executable_assets.is_empty() {
            return Ok(Vec::new());
        }
        let sandbox = self
            .sandbox
            .clone()
            .context("no namespace isolation backend is available")?;
        let store = corpus::extension::store::PackageStore::new(self.store_root.clone())?;
        let mut prepared = Vec::new();
        for asset in snapshot.executable_assets.iter() {
            let activation = snapshot
                .activation_records
                .get(&asset.package_id)
                .with_context(|| {
                    format!(
                        "executable package '{}' has no activation authority",
                        asset.package_id
                    )
                })?;
            anyhow::ensure!(
                activation.granted_permissions.executables
                    && activation.permission_approval.is_some(),
                "executable asset has no permission approval"
            );
            let manifest = parse_executable_runtime_manifest(
                &std::fs::read_to_string(&asset.absolute_path).with_context(|| {
                    format!("reading runtime manifest {}", asset.absolute_path.display())
                })?,
            )?;
            anyhow::ensure!(
                manifest.secret_refs.is_empty(),
                "runtime secret references require a configured secret approval resolver"
            );
            anyhow::ensure!(
                !manifest.isolation.network || activation.granted_permissions.network,
                "runtime requests unapproved network access"
            );
            let granted_filesystem = activation
                .granted_permissions
                .filesystem
                .clone()
                .unwrap_or_default();
            anyhow::ensure!(
                manifest
                    .isolation
                    .filesystem
                    .iter()
                    .all(|path| granted_filesystem.contains(path)),
                "runtime requests unapproved filesystem access"
            );
            let package_root = store.package_path(&asset.package_hash)?.canonicalize()?;
            let command = package_root.join(&manifest.command).canonicalize()?;
            anyhow::ensure!(
                command.starts_with(&package_root) && command.is_file(),
                "runtime command escapes its package or is not a file"
            );
            let workdir = self
                .data_root
                .join("extension-runtimes")
                .join(&asset.package_hash)
                .join(&manifest.id);
            std::fs::create_dir_all(&workdir)?;
            let mut writable = vec![workdir.clone()];
            for path in &granted_filesystem {
                let path = std::path::PathBuf::from(path);
                anyhow::ensure!(
                    path.is_absolute(),
                    "approved filesystem path is not absolute"
                );
                writable.push(path);
            }
            let workspace = WorkspacePolicy::from_resolved_roots(workdir.clone(), writable.clone())
                .map_err(anyhow::Error::msg)?;
            let provider = Arc::new(
                crate::extensions::subprocess::SubprocessAgentRuntimeProvider::new(
                    crate::extensions::subprocess::SubprocessConfig {
                        command: command.to_string_lossy().into_owned(),
                        args: manifest.args,
                        working_dir: Some(workdir.to_string_lossy().into_owned()),
                        cpu_time_seconds: manifest.isolation.cpu_time_seconds,
                        memory_bytes: manifest.isolation.memory_bytes,
                        max_processes: manifest.isolation.max_processes,
                        ..Default::default()
                    },
                    sandbox.clone(),
                    SandboxConfig {
                        workspace,
                        environment: Default::default(),
                        policy: Some(ResolvedSandboxPolicy {
                            name: format!("extension:{}", manifest.id),
                            read_only_roots: vec![
                                "/usr".into(),
                                "/lib".into(),
                                "/lib64".into(),
                                "/bin".into(),
                                "/etc".into(),
                                package_root.clone(),
                            ],
                            read_write_roots: writable,
                            deny_exact: Vec::new(),
                            deny_globs: vec![
                                "**/*.pem".into(),
                                "**/.env".into(),
                                "**/credentials*".into(),
                            ],
                            restrict_network: !manifest.isolation.network,
                        }),
                    },
                )?,
            );
            provider.probe().await?;
            prepared.push(PreparedExecutable {
                owner: asset.package_id.clone(),
                id: RuntimeId(manifest.id),
                provider: provider as Arc<dyn runtime::AgentRuntimeProvider>,
            });
        }
        Ok(prepared)
    }
}

struct PreparedExecutable {
    owner: String,
    id: ::contracts::RuntimeId,
    provider: Arc<dyn runtime::AgentRuntimeProvider>,
}

pub struct ExtensionRecoveryOutcome {
    pub snapshot: crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot,
    pub quarantined: Vec<String>,
    pub rolled_back: Vec<String>,
}

/// Rebuild the enabled package set in stable package-ID order. Each package is
/// admitted only after the accumulated snapshot compiles and probes, so one
/// corrupt activation cannot make the daemon or unrelated packages unavailable.
pub async fn reconcile_extension_snapshot(
    store_root: &Path,
    compiler: &crate::extensions::extension_snapshot::ExtensionSnapshotCompiler,
    publisher: &dyn crate::extensions::extension_coordinator::ExtensionRuntimePublisher,
    previous: Arc<crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot>,
) -> anyhow::Result<ExtensionRecoveryOutcome> {
    use corpus::extension::resolver::{PackageAssetResolver, ResolvedPackageSet};
    use corpus::extension::store::PackageStore;

    let store = PackageStore::new(store_root.to_owned())?;
    let resolver = PackageAssetResolver::from_root(store_root)?;
    let mut activations = store.list_activations()?;
    activations.sort_by(|left, right| left.package_id.cmp(&right.package_id));
    let mut resolved = ResolvedPackageSet::default();
    let mut snapshot = compiler.compile(&resolved)?;
    let mut rolled_back = Vec::new();

    for activation in activations.iter().filter(|activation| activation.enabled) {
        match admit_activation(&resolver, compiler, publisher, &resolved, activation).await {
            Ok((candidate_resolved, candidate_snapshot)) => {
                resolved = candidate_resolved;
                snapshot = candidate_snapshot;
            }
            Err(candidate_error) => {
                let mut recovered = false;
                if let Some(previous_hash) = activation
                    .previous_known_good
                    .clone()
                    .filter(|hash| Some(hash) != activation.current.as_ref())
                {
                    let mut rollback = activation.clone();
                    rollback.current = Some(previous_hash);
                    if let Ok((candidate_resolved, candidate_snapshot)) =
                        admit_activation(&resolver, compiler, publisher, &resolved, &rollback).await
                    {
                        rollback.previous_known_good = activation.current.clone();
                        rollback.health = "rolled_back".into();
                        rollback.quarantine_reason = Some(format!("{candidate_error:#}"));
                        store.write_activation(&rollback)?;
                        append_recovery_evidence(
                            &store,
                            "successful_recovery",
                            &rollback.package_id,
                            "rolled_back",
                            rollback.current.as_deref(),
                        )?;
                        rolled_back.push(rollback.package_id.clone());
                        resolved = candidate_resolved;
                        snapshot = candidate_snapshot;
                        recovered = true;
                    }
                }
                if !recovered {
                    let mut quarantined = activation.clone();
                    quarantined.enabled = false;
                    quarantined.health = "quarantined".into();
                    quarantined.quarantine_reason = Some(format!("{candidate_error:#}"));
                    store.write_activation(&quarantined)?;
                    append_recovery_evidence(
                        &store,
                        "degraded_health",
                        &quarantined.package_id,
                        "quarantined",
                        quarantined.current.as_deref(),
                    )?;
                }
            }
        }
    }

    publisher.probe(&snapshot).await?;
    publisher
        .publish(previous, Arc::new(snapshot.clone()))
        .await?;
    let mut quarantined = store
        .list_activations()?
        .into_iter()
        .filter(|activation| activation.health == "quarantined")
        .map(|activation| activation.package_id)
        .collect::<Vec<_>>();
    quarantined.sort();
    rolled_back.sort();
    rolled_back.dedup();
    Ok(ExtensionRecoveryOutcome {
        snapshot,
        quarantined,
        rolled_back,
    })
}

async fn admit_activation(
    resolver: &corpus::extension::resolver::PackageAssetResolver,
    compiler: &crate::extensions::extension_snapshot::ExtensionSnapshotCompiler,
    publisher: &dyn crate::extensions::extension_coordinator::ExtensionRuntimePublisher,
    retained: &corpus::extension::resolver::ResolvedPackageSet,
    activation: &corpus::extension::store::ActivationRecord,
) -> anyhow::Result<(
    corpus::extension::resolver::ResolvedPackageSet,
    crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot,
)> {
    let package = resolver.resolve_activation(activation)?;
    let mut candidate = retained.clone();
    candidate.assets.extend(package.assets);
    candidate
        .activation_records
        .extend(package.activation_records);
    let snapshot = compiler.compile(&candidate)?;
    publisher.probe(&snapshot).await?;
    Ok((candidate, snapshot))
}

fn append_recovery_evidence(
    store: &corpus::extension::store::PackageStore,
    event_type: &str,
    package_id: &str,
    result: &str,
    hash: Option<&str>,
) -> anyhow::Result<()> {
    store.append_evidence(&corpus::extension::store::ExtensionEvidenceEvent {
        schema_version: 1,
        event_type: event_type.into(),
        correlation_id: uuid::Uuid::new_v4().to_string(),
        package_id: package_id.into(),
        package_version: None,
        result: result.into(),
        evidence_references: hash
            .map(|hash| vec![format!("package:sha256:{hash}")])
            .unwrap_or_default(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub(super) struct RuntimeExtensionIndex {
    pub catalog: corpus::ExtensionCatalog,
    pub ids: Vec<corpus::ExtensionId>,
    pub capabilities: Vec<::contracts::CapabilityId>,
}

pub(super) fn index_runtime_extensions(
    skills: &corpus::SkillLoader,
    hooks: &corpus::HookRegistry,
) -> anyhow::Result<RuntimeExtensionIndex> {
    let descriptors = corpus::discover_runtime_extensions(skills, hooks)?;
    let ids = descriptors
        .iter()
        .map(|descriptor| descriptor.id.clone())
        .collect();
    let capabilities = descriptors
        .iter()
        .flat_map(|descriptor| descriptor.capabilities.clone())
        .collect();
    Ok(RuntimeExtensionIndex {
        catalog: corpus::ExtensionCatalog::new(descriptors)?,
        ids,
        capabilities,
    })
}

pub(super) async fn activate_runtime_extensions(
    corpus: Arc<dyn corpus::CorpusService>,
    ids: Vec<corpus::ExtensionId>,
    capabilities: Vec<::contracts::CapabilityId>,
    state_root: &Path,
    session_id: &str,
) -> anyhow::Result<Arc<dyn crate::extensions::extension_service::ExtensionDecisionSink>> {
    let decisions: Arc<dyn crate::extensions::extension_service::ExtensionDecisionSink> = Arc::new(
        crate::extensions::extension_service::SpineExtensionDecisionSink::new(Arc::new(
            adapters_sqlite::event_spine::SqliteEventSpine::open(
                state_root.join("extension-events.db"),
            )
            .unwrap_or_else(|_| {
                adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:")
                    .expect("in-memory extension decision spine")
            }),
        )),
    );
    let activation = crate::extensions::ExtensionService::new(corpus, decisions.clone())
        .activate(
            corpus::ExtensionGrant {
                grant_id: format!("runtime-extensions:{session_id}"),
                principal: ::contracts::PrincipalId(application::LOCAL_OWNER_PRINCIPAL.into()),
                session_id: session_id.into(),
                agent_id: None,
                capabilities,
                resources: ::contracts::CapabilityScope::default(),
            },
            ids,
            &crate::extensions::SessionExtensionPolicy::default(),
        )
        .await?;
    tracing::info!(
        count = activation.receipt.extensions.len(),
        "Runtime skills, plugins, and hooks activated through scoped catalog"
    );
    Ok(decisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::extension::store::{
        ActivationRecord, InstalledPackageRecord, PackageStore, PermissionApprovalRecord,
    };
    use corpus::tools::tools::skill_tools::SharedSkills;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    const HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const OLD_HASH: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const BAD_HASH: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    async fn recover(
        store_root: &Path,
        data_root: &Path,
    ) -> (
        ExtensionRecoveryOutcome,
        Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter>,
    ) {
        let publisher = Arc::new(
            crate::wiring::daemon::bootstrap::extension_publisher::DaemonExtensionRuntimePublisher::new(
                Arc::new(tokio::sync::Mutex::new(corpus::ToolRegistry::new())),
                Arc::new(tokio::sync::Mutex::new(corpus::HookRegistry::new(Arc::new(
                    kernel::chronos::TestClock::default(),
                )))),
                SharedSkills::new(Arc::new(Vec::new())),
            ),
        );
        let router = publisher
            .bind_executable_runtime(
                data_root,
                store_root,
                Arc::new(kernel::chronos::TestClock::default()),
            )
            .await
            .unwrap();
        let outcome = reconcile_extension_snapshot(
            store_root,
            &crate::extensions::extension_snapshot::ExtensionSnapshotCompiler::default(),
            publisher.as_ref(),
            Arc::new(crate::extensions::extension_snapshot::ExtensionRuntimeSnapshot::empty()),
        )
        .await
        .unwrap();
        (outcome, router)
    }

    fn put_runtime_package(store: &PackageStore, hash: &str, version: &str, script: &str) {
        let package = store.package_path(hash).unwrap();
        std::fs::create_dir_all(package.join("assets/executables/generic")).unwrap();
        std::fs::create_dir_all(package.join("payload")).unwrap();
        std::fs::write(
            package.join("assets/executables/generic/runtime.toml"),
            r#"
schema_version = 1
id = "runtime.generic"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "payload/runtime.py"
[isolation]
network = false
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
        let command = package.join("payload/runtime.py");
        std::fs::write(&command, script).unwrap();
        let mut permissions = std::fs::metadata(&command).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&command, permissions).unwrap();
        store
            .put_installed(&InstalledPackageRecord {
                schema_version: 1,
                id: "test.runtime".into(),
                version: version.into(),
                description: "test".into(),
                hash: hash.into(),
                file_count: 2,
                total_size: 1,
                installed_at: format!("2026-07-24T{version}:00Z"),
                assets: vec![corpus::extension::package::AssetRef {
                    kind: corpus::extension::asset::AssetKind::Executable,
                    id: "runtime.generic".into(),
                    path: "assets/executables/generic/runtime.toml".into(),
                }],
                requested_permissions: corpus::extension::package::PermissionRequestSet {
                    filesystem: None,
                    network: false,
                    executables: true,
                },
                source: corpus::extension::store::PackageSourceRecord::LocalArchive,
                workspace_trust: None,
            })
            .unwrap();
    }

    #[tokio::test]
    async fn enabled_package_runtime_is_probed_and_registered_in_daemon_router() {
        let temp = TempDir::new().unwrap();
        let store_root = temp.path().join("store");
        let data_root = temp.path().join("state");
        let store = PackageStore::new(store_root.clone()).unwrap();
        let package = store.package_path(HASH).unwrap();
        std::fs::create_dir_all(package.join("assets/executables/generic")).unwrap();
        std::fs::create_dir_all(package.join("payload")).unwrap();
        std::fs::write(
            package.join("assets/executables/generic/runtime.toml"),
            r#"
schema_version = 1
id = "runtime.generic"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "payload/runtime.py"
[isolation]
network = false
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
        let source = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/extension_jsonrpc_runtime.py"),
        )
        .unwrap()
        .replacen("#!/usr/bin/env python3", "#!/usr/bin/python3", 1);
        let command = package.join("payload/runtime.py");
        std::fs::write(&command, source).unwrap();
        let mut permissions = std::fs::metadata(&command).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&command, permissions).unwrap();
        let requested = corpus::extension::package::PermissionRequestSet {
            filesystem: None,
            network: false,
            executables: true,
        };
        store
            .put_installed(&InstalledPackageRecord {
                schema_version: 1,
                id: "test.runtime".into(),
                version: "1.0.0".into(),
                description: "test".into(),
                hash: HASH.into(),
                file_count: 2,
                total_size: 1,
                installed_at: "2026-07-24T00:00:00Z".into(),
                assets: vec![corpus::extension::package::AssetRef {
                    kind: corpus::extension::asset::AssetKind::Executable,
                    id: "runtime.generic".into(),
                    path: "assets/executables/generic/runtime.toml".into(),
                }],
                requested_permissions: requested.clone(),
                source: corpus::extension::store::PackageSourceRecord::LocalArchive,
                workspace_trust: None,
            })
            .unwrap();
        store
            .write_activation(&ActivationRecord {
                schema_version: 1,
                package_id: "test.runtime".into(),
                enabled: true,
                current: Some(HASH.into()),
                previous_known_good: None,
                granted_permissions: requested.clone(),
                permission_approval: Some(PermissionApprovalRecord {
                    actor: "operator:test".into(),
                    approved_at: "2026-07-24T00:00:00Z".into(),
                    permissions: requested,
                }),
                activated_assets: vec!["runtime.generic".into()],
                health: "healthy".into(),
                quarantine_reason: None,
            })
            .unwrap();

        let (composition, router) = recover(&store_root, &data_root).await;
        assert!(composition.quarantined.is_empty());
        assert!(composition.rolled_back.is_empty());
        assert_eq!(
            router.registered(),
            vec![::contracts::RuntimeId("runtime.generic".into())]
        );
    }

    #[tokio::test]
    async fn failed_candidate_is_automatically_rolled_back_and_reprobed() {
        let temp = TempDir::new().unwrap();
        let store_root = temp.path().join("store");
        let data_root = temp.path().join("state");
        let store = PackageStore::new(store_root.clone()).unwrap();
        let helper = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/extension_jsonrpc_runtime.py"),
        )
        .unwrap()
        .replacen("#!/usr/bin/env python3", "#!/usr/bin/python3", 1);
        put_runtime_package(&store, OLD_HASH, "01", &helper);
        put_runtime_package(
            &store,
            BAD_HASH,
            "02",
            "#!/usr/bin/python3\nraise SystemExit(23)\n",
        );
        let requested = corpus::extension::package::PermissionRequestSet {
            filesystem: None,
            network: false,
            executables: true,
        };
        store
            .write_activation(&ActivationRecord {
                schema_version: 1,
                package_id: "test.runtime".into(),
                enabled: true,
                current: Some(BAD_HASH.into()),
                previous_known_good: Some(OLD_HASH.into()),
                granted_permissions: requested.clone(),
                permission_approval: Some(PermissionApprovalRecord {
                    actor: "operator:test".into(),
                    approved_at: "2026-07-24T00:00:00Z".into(),
                    permissions: requested,
                }),
                activated_assets: vec!["runtime.generic".into()],
                health: "healthy".into(),
                quarantine_reason: None,
            })
            .unwrap();
        let (composition, router) = recover(&store_root, &data_root).await;
        assert_eq!(
            router.registered(),
            vec![::contracts::RuntimeId("runtime.generic".into())]
        );
        assert_eq!(composition.rolled_back, vec!["test.runtime"]);
        let state = store.read_activation("test.runtime").unwrap();
        assert!(state.enabled);
        assert_eq!(state.current.as_deref(), Some(OLD_HASH));
        assert_eq!(state.previous_known_good.as_deref(), Some(BAD_HASH));
        assert_eq!(state.health, "rolled_back");
        assert!(!state.quarantine_reason.unwrap().is_empty());
    }
}
