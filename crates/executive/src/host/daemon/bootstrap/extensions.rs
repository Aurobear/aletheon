//! Scoped composition of non-tool Corpus extensions.

use std::path::Path;
use std::sync::Arc;
use anyhow::Context;

pub(super) struct ExtensionRuntimeComposition {
    pub router: Arc<crate::application::extension_runtime_router::ExtensionRuntimeRouter>,
    pub quarantined: Vec<String>,
}

pub(super) struct RuntimeExtensionIndex {
    pub catalog: corpus::ExtensionCatalog,
    pub ids: Vec<fabric::ExtensionId>,
    pub capabilities: Vec<fabric::CapabilityId>,
}

pub(super) async fn register_package_runtimes(
    agent_runtimes: &crate::application::agent_control::AgentRuntimeRegistry,
    data_root: &Path,
    store_root: &Path,
    clock: Arc<dyn fabric::Clock>,
) -> anyhow::Result<ExtensionRuntimeComposition> {
    use corpus::extension::manifest::parse_executable_runtime_manifest;
    use corpus::extension::store::PackageStore;
    use fabric::{ResolvedSandboxPolicy, RuntimeId, SandboxConfig, WorkspacePolicy};

    let router = Arc::new(
        crate::application::extension_runtime_router::ExtensionRuntimeRouter::default(),
    );
    let store = PackageStore::new(store_root.to_owned())?;
    let sandbox = corpus::security::sandbox::BubblewrapBackend::probe_async(clock)
        .await
        .map(|backend| Arc::new(backend) as Arc<dyn fabric::SandboxBackend>);
    let mut quarantined = Vec::new();

    for installed in store.list_installed()? {
        let mut activation = store.read_activation(&installed.id)?;
        if !activation.enabled || activation.current.as_deref() != Some(&installed.hash) {
            continue;
        }
        for asset in installed
            .assets
            .iter()
            .filter(|asset| asset.kind == fabric::AssetKind::Executable)
        {
            let registration = async {
                anyhow::ensure!(
                    activation.granted_permissions.executables
                        && activation.permission_approval.is_some(),
                    "executable asset has no permission approval"
                );
                let sandbox = sandbox
                    .clone()
                    .context("no namespace isolation backend is available")?;
                let package_root = store.package_path(&installed.hash)?;
                let manifest_path = package_root.join(&asset.path);
                let manifest = parse_executable_runtime_manifest(
                    &std::fs::read_to_string(&manifest_path).with_context(|| {
                        format!("reading runtime manifest {}", manifest_path.display())
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
                let command = package_root.join(&manifest.command).canonicalize()?;
                anyhow::ensure!(
                    command.starts_with(package_root.canonicalize()?) && command.is_file(),
                    "runtime command escapes its package or is not a file"
                );

                let workdir = data_root
                    .join("extension-runtimes")
                    .join(&installed.hash)
                    .join(&manifest.id);
                std::fs::create_dir_all(&workdir)?;
                let mut writable = vec![workdir.clone()];
                for path in &granted_filesystem {
                    let path = std::path::PathBuf::from(path);
                    anyhow::ensure!(path.is_absolute(), "approved filesystem path is not absolute");
                    writable.push(path);
                }
                let workspace =
                    WorkspacePolicy::from_resolved_roots(workdir.clone(), writable.clone())
                        .map_err(anyhow::Error::msg)?;
                let policy = ResolvedSandboxPolicy {
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
                };
                let runtime_config =
                    crate::extensions::runtime::subprocess::SubprocessConfig {
                        command: command.to_string_lossy().into_owned(),
                        args: manifest.args,
                        working_dir: Some(workdir.to_string_lossy().into_owned()),
                        cpu_time_seconds: manifest.isolation.cpu_time_seconds,
                        memory_bytes: manifest.isolation.memory_bytes,
                        max_processes: manifest.isolation.max_processes,
                        ..Default::default()
                    };
                let provider = Arc::new(
                    crate::extensions::runtime::subprocess::SubprocessAgentRuntimeProvider::new(
                        runtime_config,
                        sandbox,
                        SandboxConfig {
                            workspace,
                            environment: Default::default(),
                            policy: Some(policy),
                        },
                    )?,
                );
                provider.probe().await?;
                let runtime_id = RuntimeId(manifest.id);
                router.register(runtime_id.clone(), provider)?;
                agent_runtimes.register(
                    runtime_id,
                    Arc::new(
                        crate::application::extension_runtime_router::ExtensionProviderLauncher::new(
                            router.clone(),
                        ),
                    ),
                )?;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            if let Err(error) = registration {
                let reason = format!("{}:{}: {error:#}", installed.id, asset.id);
                tracing::error!(reason = %reason, "Extension runtime quarantined");
                activation.enabled = false;
                activation.health = "quarantined".into();
                activation.quarantine_reason = Some(reason.clone());
                store.write_activation(&activation)?;
                quarantined.push(reason);
            }
        }
    }
    Ok(ExtensionRuntimeComposition {
        router,
        quarantined,
    })
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
    ids: Vec<fabric::ExtensionId>,
    capabilities: Vec<fabric::CapabilityId>,
    state_root: &Path,
    session_id: &str,
) -> anyhow::Result<Arc<dyn crate::application::extension_service::ExtensionDecisionSink>> {
    let decisions: Arc<dyn crate::application::extension_service::ExtensionDecisionSink> = Arc::new(
        crate::application::extension_service::SpineExtensionDecisionSink::new(Arc::new(
            crate::adapters::events::SqliteEventSpine::open(state_root.join("extension-events.db"))
                .unwrap_or_else(|_| {
                    crate::adapters::events::SqliteEventSpine::open(":memory:")
                        .expect("in-memory extension decision spine")
                }),
        )),
    );
    let activation = crate::application::ExtensionService::new(corpus, decisions.clone())
        .activate(
            corpus::ExtensionGrant {
                grant_id: format!("runtime-extensions:{session_id}"),
                principal: fabric::PrincipalId(fabric::LOCAL_OWNER_PRINCIPAL.into()),
                session_id: session_id.into(),
                agent_id: None,
                capabilities,
                resources: fabric::CapabilityScope::default(),
            },
            ids,
            &crate::application::SessionExtensionPolicy::default(),
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
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    const HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

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
        let requested = fabric::PermissionRequestSet {
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
                assets: vec![fabric::AssetRef {
                    kind: fabric::AssetKind::Executable,
                    id: "runtime.generic".into(),
                    path: "assets/executables/generic/runtime.toml".into(),
                }],
                requested_permissions: requested.clone(),
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

        let registry =
            crate::application::agent_control::AgentRuntimeRegistry::default();
        let composition = register_package_runtimes(
            &registry,
            &data_root,
            &store_root,
            Arc::new(kernel::chronos::TestClock::default()),
        )
        .await
        .unwrap();
        assert!(composition.quarantined.is_empty());
        assert_eq!(
            composition.router.registered(),
            vec![fabric::RuntimeId("runtime.generic".into())]
        );
        assert!(registry
            .resolve(&fabric::RuntimeId("runtime.generic".into()))
            .is_ok());
    }
}
