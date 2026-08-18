//! Staged composition of package-owned runtime assets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ::contracts::{Clock, LlmProvider};
use corpus::tools::tools::skill_tools::SharedSkills;
use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;
use tokio::sync::Mutex;

use crate::config::CognitiveRuntimeConfig;
use crate::extensions::extension_coordinator::ExtensionCoordinator;
use crate::extensions::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
use cognit::ports::inference::InferencePort;

use super::extension_publisher::{DaemonExtensionRuntimePublisher, PackageProfileRuntime};
use super::extensions::{reconcile_extension_snapshot, ExtensionRecoveryOutcome};

pub(super) struct PendingExtensionRuntime {
    store_root: PathBuf,
    compiler: ExtensionSnapshotCompiler,
    publisher: Arc<DaemonExtensionRuntimePublisher>,
    view: ExtensionRuntimeView,
    initial_recovery: ExtensionRecoveryOutcome,
}

pub(super) struct ExtensionBootstrap {
    pub coordinator: Arc<ExtensionCoordinator>,
    publisher: Arc<DaemonExtensionRuntimePublisher>,
    pub view: ExtensionRuntimeView,
    pub runtime_count: u64,
    pub quarantined_ids: Vec<String>,
    pub rolled_back: Vec<String>,
    pub package_count: u64,
    pub snapshot_digest: String,
    pub asset_counts: BTreeMap<String, u64>,
}

impl ExtensionBootstrap {
    pub async fn bind_runtime_agent_supervisor(
        &self,
        supervisor: Arc<runtime::RuntimeAgentSupervisor>,
        backend: Arc<dyn runtime::DelegateBackend>,
        agent_host: std::sync::Weak<dyn crate::composition::agent_control::AgentHostEffects>,
    ) -> anyhow::Result<()> {
        self.publisher
            .bind_runtime_agent_supervisor(supervisor, backend, agent_host)
            .await
    }
}

impl PendingExtensionRuntime {
    pub async fn prepare(
        tools: Arc<Mutex<ToolRegistry>>,
        hooks: Arc<Mutex<HookRegistry>>,
        skills: SharedSkills,
        configured_connectors: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<Self> {
        let view = ExtensionRuntimeView::default();
        let publisher = Arc::new(DaemonExtensionRuntimePublisher::new(tools, hooks, skills));
        let store_root = corpus::extension::store::PackageStore::configured_user_root();
        let compiler = ExtensionSnapshotCompiler::new(configured_connectors);
        let initial_recovery = reconcile_extension_snapshot(
            &store_root,
            &compiler,
            publisher.as_ref(),
            view.load().await,
        )
        .await?;
        view.publish(initial_recovery.snapshot.clone()).await;
        Ok(Self {
            store_root,
            compiler,
            publisher,
            view,
            initial_recovery,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish(
        self,
        data_root: &Path,
        clock: Arc<dyn Clock>,
        profiles: Arc<crate::host::runtime::AgentProfileRegistry>,
        inference: Arc<dyn InferencePort>,
        llm: Arc<dyn LlmProvider>,
        config: CognitiveRuntimeConfig,
    ) -> anyhow::Result<ExtensionBootstrap> {
        self.publisher
            .bind_profiles(PackageProfileRuntime::new(profiles, inference, llm, config))
            .await?;
        let runtime_router = self
            .publisher
            .bind_executable_runtime(
                data_root,
                &corpus::extension::store::PackageStore::configured_user_root(),
                clock.clone(),
            )
            .await?;
        let final_recovery = reconcile_extension_snapshot(
            &self.store_root,
            &self.compiler,
            self.publisher.as_ref(),
            self.view.load().await,
        )
        .await?;
        let snapshot = final_recovery.snapshot;
        self.view.publish(snapshot.clone()).await;
        let publisher = self.publisher.clone();
        let coordinator = Arc::new(ExtensionCoordinator::new(
            &self.store_root,
            self.compiler,
            publisher.clone(),
            self.view.clone(),
            clock,
        )?);
        Ok(ExtensionBootstrap::from_recovery(
            coordinator,
            self.view,
            publisher,
            runtime_router.registered().len() as u64,
            self.initial_recovery,
            final_recovery.quarantined,
            final_recovery.rolled_back,
            snapshot,
        ))
    }
}

impl ExtensionBootstrap {
    pub fn publish_health(&self, registry: &application::health::HealthRegistry) {
        use application::health::ComponentHealth;

        let mut runtimes = ComponentHealth::ready();
        runtimes.count = Some(self.runtime_count);
        registry.set("extension_runtimes", runtimes);
        let mut rollbacks = if self.rolled_back.is_empty() {
            ComponentHealth::ready()
        } else {
            ComponentHealth::degraded("extension_rolled_back")
        };
        rollbacks.count = Some(self.rolled_back.len() as u64);
        rollbacks.items = self.rolled_back.clone();
        registry.set("extension_rollbacks", rollbacks);
        let mut packages = if self.quarantined_ids.is_empty() {
            ComponentHealth::ready()
        } else {
            ComponentHealth::degraded("extension_packages_quarantined")
        };
        packages.count = Some(self.package_count);
        packages.items = self.quarantined_ids.clone();
        packages.snapshot_digest = Some(self.snapshot_digest.clone());
        packages.asset_counts = self.asset_counts.clone();
        registry.set("extension_packages", packages);
    }

    fn from_recovery(
        coordinator: Arc<ExtensionCoordinator>,
        view: ExtensionRuntimeView,
        publisher: Arc<DaemonExtensionRuntimePublisher>,
        runtime_count: u64,
        initial: ExtensionRecoveryOutcome,
        final_quarantined: Vec<String>,
        final_rolled_back: Vec<String>,
        snapshot: ExtensionRuntimeSnapshot,
    ) -> Self {
        let mut quarantined_ids = initial.quarantined;
        quarantined_ids.extend(final_quarantined);
        quarantined_ids.sort();
        quarantined_ids.dedup();
        let mut rolled_back = initial.rolled_back;
        rolled_back.extend(final_rolled_back);
        rolled_back.sort();
        rolled_back.dedup();
        let asset_counts = BTreeMap::from([
            ("skills".to_owned(), snapshot.skills.len() as u64),
            ("hooks".to_owned(), snapshot.hooks.len() as u64),
            ("connectors".to_owned(), snapshot.connectors.len() as u64),
            (
                "agent_profiles".to_owned(),
                snapshot.agent_profiles.len() as u64,
            ),
            (
                "executables".to_owned(),
                snapshot.executable_assets.len() as u64,
            ),
        ]);
        Self {
            coordinator,
            publisher,
            view,
            runtime_count,
            quarantined_ids,
            rolled_back,
            package_count: snapshot.package_digests.len() as u64,
            snapshot_digest: snapshot.digest.clone(),
            asset_counts,
        }
    }
}
