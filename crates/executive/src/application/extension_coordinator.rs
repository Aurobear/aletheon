//! Daemon-owned transaction boundary for extension lifecycle mutations.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use corpus::extension::resolver::PackageAssetResolver;
use corpus::extension::store::{
    ActivationRecord, ExtensionEvidenceEvent, InstalledPackageRecord, PackageStore,
};
use fabric::protocol::extension::{ExtensionMutationReceiptV1, EXTENSION_PROTOCOL_SCHEMA_V1};
use tokio::sync::Mutex;

use super::extension_install::ExtensionInstallService;
use super::extension_manage::ExtensionManageService;
use super::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionRuntimeView, ExtensionSnapshotCompiler,
};

#[async_trait]
pub trait ExtensionRuntimePublisher: Send + Sync {
    async fn probe(&self, candidate: &ExtensionRuntimeSnapshot) -> Result<()>;

    async fn publish(
        &self,
        previous: Arc<ExtensionRuntimeSnapshot>,
        candidate: Arc<ExtensionRuntimeSnapshot>,
    ) -> Result<()>;
}

/// Serializes Package Store changes with candidate compilation and runtime publication.
pub struct ExtensionCoordinator {
    mutations: Mutex<()>,
    install: ExtensionInstallService,
    manage: ExtensionManageService,
    store_root: std::path::PathBuf,
    compiler: ExtensionSnapshotCompiler,
    publisher: Arc<dyn ExtensionRuntimePublisher>,
    view: ExtensionRuntimeView,
    clock: Arc<dyn fabric::Clock>,
}

impl ExtensionCoordinator {
    pub fn new(
        store_root: &Path,
        compiler: ExtensionSnapshotCompiler,
        publisher: Arc<dyn ExtensionRuntimePublisher>,
        view: ExtensionRuntimeView,
        clock: Arc<dyn fabric::Clock>,
    ) -> Result<Self> {
        Ok(Self {
            mutations: Mutex::new(()),
            install: ExtensionInstallService::new(store_root)?,
            manage: ExtensionManageService::new(store_root)?,
            store_root: store_root.to_owned(),
            compiler,
            publisher,
            view,
            clock,
        })
    }

    pub fn view(&self) -> ExtensionRuntimeView {
        self.view.clone()
    }

    pub fn activation(&self, package_id: &str) -> Result<ActivationRecord> {
        self.store()?.read_activation(package_id)
    }

    pub fn list(&self) -> Result<Vec<InstalledPackageRecord>> {
        self.install.list()
    }

    pub fn show(&self, package_id: &str) -> Result<Vec<InstalledPackageRecord>> {
        self.install.show(package_id)
    }

    pub fn doctor(
        &self,
        package_id: &str,
    ) -> Result<super::extension_manage::ExtensionDoctorResult> {
        self.manage.doctor(package_id)
    }

    pub async fn install(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
    ) -> Result<ExtensionMutationReceiptV1> {
        validate_actor(actor)?;
        let _guard = self.mutations.lock().await;
        let inspection = self.install.inspect(package_path)?;
        let package_id = inspection.manifest.package.id.0.clone();
        let package_version = inspection.manifest.package.version.0.clone();
        let previous = self.view.load().await;
        let install_result = if trust_workspace {
            self.install
                .install_with_workspace_trust(package_path, Some(actor))
        } else {
            self.install.install(package_path)
        };
        let package_hash = match install_result {
            Ok(hash) => hash,
            Err(error) => {
                self.record_failure("install", &package_id, Some(&package_version), &error)?;
                return Err(error);
            }
        };
        let candidate = match self.compile_candidate() {
            Ok(candidate) => candidate,
            Err(error) => {
                self.record_failure("install", &package_id, Some(&package_version), &error)?;
                return Err(error).context("compiling extension snapshot after install");
            }
        };
        if candidate.digest != previous.digest {
            let error = anyhow::anyhow!(
                "install changed active snapshot digest from {} to {}",
                previous.digest,
                candidate.digest
            );
            self.record_failure("install", &package_id, Some(&package_version), &error)?;
            return Err(error);
        }

        self.complete_receipt(
            "install",
            actor,
            &package_id,
            Some(package_version),
            Some(package_hash),
            &previous.digest,
            &candidate.digest,
            false,
            "healthy",
        )
    }

    pub async fn enable(
        &self,
        actor: &str,
        package_id: &str,
        approve_permissions: bool,
    ) -> Result<ExtensionMutationReceiptV1> {
        self.activation_mutation("enable", actor, package_id, approve_permissions, || {
            if approve_permissions {
                self.manage.enable_with_operator_approval(package_id, actor)
            } else {
                self.manage.enable(package_id)
            }
        })
        .await
    }

    pub async fn disable(
        &self,
        actor: &str,
        package_id: &str,
    ) -> Result<ExtensionMutationReceiptV1> {
        self.activation_mutation("disable", actor, package_id, false, || {
            self.manage.disable(package_id)
        })
        .await
    }

    pub async fn upgrade(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
        approve_permissions: bool,
    ) -> Result<ExtensionMutationReceiptV1> {
        validate_actor(actor)?;
        let inspection = self.install.inspect(package_path)?;
        let package_id = inspection.manifest.package.id.0;
        self.activation_mutation("upgrade", actor, &package_id, approve_permissions, || {
            let workspace_actor = trust_workspace.then_some(actor);
            if approve_permissions {
                self.manage
                    .upgrade_with_operator_approval(package_path, workspace_actor, actor)
            } else {
                self.manage
                    .upgrade_with_workspace_trust(package_path, workspace_actor)
            }
        })
        .await
    }

    pub async fn rollback(
        &self,
        actor: &str,
        package_id: &str,
    ) -> Result<ExtensionMutationReceiptV1> {
        self.activation_mutation("rollback", actor, package_id, false, || {
            self.manage.rollback(package_id)
        })
        .await
    }

    pub async fn remove(
        &self,
        actor: &str,
        package_id: &str,
    ) -> Result<ExtensionMutationReceiptV1> {
        self.activation_mutation("remove", actor, package_id, false, || {
            self.manage.remove(package_id)
        })
        .await
    }

    /// Permanently delete only an already inactive package. Requiring an
    /// explicit prior disable/remove keeps runtime publication reversible.
    pub async fn purge(&self, actor: &str, package_id: &str) -> Result<ExtensionMutationReceiptV1> {
        validate_actor(actor)?;
        anyhow::ensure!(!package_id.trim().is_empty(), "package ID is required");
        let _guard = self.mutations.lock().await;
        let store = self.store()?;
        let activation = store.read_activation(package_id)?;
        anyhow::ensure!(
            !activation.enabled,
            "extension '{package_id}' must be disabled before purge"
        );
        let previous = self.view.load().await;
        let candidate = self
            .compile_candidate()
            .context("compiling extension snapshot before purge")?;
        anyhow::ensure!(
            candidate.digest == previous.digest,
            "inactive purge precondition found a stale runtime snapshot"
        );
        let package = current_package(&store, &activation)?;
        if let Err(error) = self.manage.purge(package_id) {
            self.record_failure("purge", package_id, None, &error)?;
            return Err(error);
        }
        self.complete_receipt(
            "purge",
            actor,
            package_id,
            package.as_ref().map(|record| record.version.clone()),
            package.as_ref().map(|record| record.hash.clone()),
            &previous.digest,
            &candidate.digest,
            false,
            "removed",
        )
    }

    async fn activation_mutation<F>(
        &self,
        operation: &str,
        actor: &str,
        package_id: &str,
        approve_permissions: bool,
        mutate: F,
    ) -> Result<ExtensionMutationReceiptV1>
    where
        F: FnOnce() -> Result<()>,
    {
        validate_actor(actor)?;
        anyhow::ensure!(!package_id.trim().is_empty(), "package ID is required");
        let _guard = self.mutations.lock().await;
        let store = self.store()?;
        let old_activation = store.read_activation(package_id)?;
        let previous = self.view.load().await;

        if let Err(error) = mutate() {
            self.restore_activation(&old_activation)?;
            self.record_failure(operation, package_id, None, &error)?;
            return Err(error);
        }

        let candidate = match self.compile_candidate() {
            Ok(candidate) => Arc::new(candidate),
            Err(error) => {
                self.restore_activation(&old_activation)?;
                self.record_failure(operation, package_id, None, &error)?;
                return Err(error).context("compiling extension snapshot");
            }
        };
        if let Err(error) = self.publisher.probe(candidate.as_ref()).await {
            self.restore_activation(&old_activation)?;
            self.record_failure(operation, package_id, None, &error)?;
            return Err(error).context("probing extension runtime candidate");
        }

        let activation = match store.read_activation(package_id) {
            Ok(activation) => activation,
            Err(error) => {
                self.restore_activation(&old_activation)?;
                self.record_failure(operation, package_id, None, &error)?;
                return Err(error).context("reading coordinated extension activation");
            }
        };
        let package = match current_package(&store, &activation) {
            Ok(package) => package,
            Err(error) => {
                self.restore_activation(&old_activation)?;
                self.record_failure(operation, package_id, None, &error)?;
                return Err(error).context("reading coordinated extension package");
            }
        };
        let package_version = package.as_ref().map(|record| record.version.clone());
        let package_hash = package.as_ref().map(|record| record.hash.clone());
        let permission_approved = approve_permissions && activation.permission_approval.is_some();
        let health = if activation.health.is_empty() {
            "healthy".to_owned()
        } else {
            activation.health.clone()
        };

        if let Err(error) = self
            .publisher
            .publish(previous.clone(), candidate.clone())
            .await
        {
            let runtime_rollback = self
                .publisher
                .publish(candidate.clone(), previous.clone())
                .await;
            self.restore_activation(&old_activation)?;
            self.record_failure(operation, package_id, None, &error)?;
            if let Err(rollback_error) = runtime_rollback {
                return Err(error).context(format!(
                    "publishing extension runtime candidate; runtime rollback also failed: {rollback_error:#}"
                ));
            }
            return Err(error).context("publishing extension runtime candidate");
        }
        self.view.publish(candidate.as_ref().clone()).await;

        let completion = self.complete_receipt(
            operation,
            actor,
            package_id,
            package_version,
            package_hash,
            &previous.digest,
            &candidate.digest,
            permission_approved,
            &health,
        );
        match completion {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                let runtime_rollback = self
                    .publisher
                    .publish(candidate.clone(), previous.clone())
                    .await;
                self.view.publish(previous.as_ref().clone()).await;
                self.restore_activation(&old_activation)?;
                let _ = self.record_failure(operation, package_id, None, &error);
                if let Err(rollback_error) = runtime_rollback {
                    return Err(error).context(format!(
                        "persisting extension mutation receipt; runtime rollback also failed: {rollback_error:#}"
                    ));
                }
                Err(error).context("persisting extension mutation receipt")
            }
        }
    }

    fn compile_candidate(&self) -> Result<ExtensionRuntimeSnapshot> {
        let resolved = PackageAssetResolver::from_root(&self.store_root)?.resolve_enabled()?;
        self.compiler.compile(&resolved)
    }

    fn restore_activation(&self, activation: &ActivationRecord) -> Result<()> {
        self.store()?
            .write_activation(activation)
            .context("restoring previous extension activation")
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_receipt(
        &self,
        operation: &str,
        actor: &str,
        package_id: &str,
        package_version: Option<String>,
        package_hash: Option<String>,
        previous_snapshot_digest: &str,
        snapshot_digest: &str,
        permission_approved: bool,
        health: &str,
    ) -> Result<ExtensionMutationReceiptV1> {
        let mut evidence_references = vec![format!("snapshot:sha256:{snapshot_digest}")];
        if let Some(hash) = &package_hash {
            evidence_references.push(format!("package:sha256:{hash}"));
        }
        let receipt = ExtensionMutationReceiptV1 {
            schema_version: EXTENSION_PROTOCOL_SCHEMA_V1,
            operation: operation.to_owned(),
            actor: actor.to_owned(),
            package_id: package_id.to_owned(),
            package_version,
            package_hash,
            previous_snapshot_digest: previous_snapshot_digest.to_owned(),
            snapshot_digest: snapshot_digest.to_owned(),
            permission_approved,
            health: health.to_owned(),
            evidence_references,
        };
        let store = self.store()?;
        let receipt_owner = if operation == "purge" {
            format!("purged:{package_id}")
        } else {
            package_id.to_owned()
        };
        let path = store.store_receipt(&receipt_owner, &serde_json::to_value(&receipt)?)?;
        let mut event_references = receipt.evidence_references.clone();
        event_references.push(format!(
            "receipt:{}",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown")
        ));
        store.append_evidence(&ExtensionEvidenceEvent {
            schema_version: 1,
            event_type: format!("extension_{operation}_coordinated"),
            correlation_id: uuid::Uuid::new_v4().to_string(),
            package_id: package_id.to_owned(),
            package_version: receipt.package_version.clone(),
            result: "succeeded".into(),
            evidence_references: event_references,
            occurred_at: fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
        })?;
        Ok(receipt)
    }

    fn record_failure(
        &self,
        operation: &str,
        package_id: &str,
        package_version: Option<&str>,
        error: &anyhow::Error,
    ) -> Result<()> {
        self.store()?.append_evidence(&ExtensionEvidenceEvent {
            schema_version: 1,
            event_type: format!("extension_{operation}_coordinated"),
            correlation_id: uuid::Uuid::new_v4().to_string(),
            package_id: package_id.to_owned(),
            package_version: package_version.map(str::to_owned),
            result: "failed".into(),
            evidence_references: vec![format!("error:{}", bounded_error(error))],
            occurred_at: fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
        })
    }

    fn store(&self) -> Result<PackageStore> {
        PackageStore::new(self.store_root.clone())
    }
}

fn current_package(
    store: &PackageStore,
    activation: &ActivationRecord,
) -> Result<Option<InstalledPackageRecord>> {
    let Some(hash) = activation.current.as_deref() else {
        return Ok(None);
    };
    Ok(store
        .get_installed(&activation.package_id)?
        .into_iter()
        .find(|record| record.hash == hash))
}

fn validate_actor(actor: &str) -> Result<()> {
    anyhow::ensure!(
        !actor.trim().is_empty(),
        "extension mutation actor is required"
    );
    anyhow::ensure!(actor.len() <= 256, "extension mutation actor is too long");
    Ok(())
}

fn bounded_error(error: &anyhow::Error) -> String {
    let value = format!("{error:#}").replace(['\n', '\r'], " ");
    value.chars().take(240).collect()
}
