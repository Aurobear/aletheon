//! Aletheon-owned extension lifecycle and runtime composition.
//!
//! Production Aletheon wiring uses this crate so package install, activation,
//! snapshot publication, and runtime routing do not enter the Aletheon
//! composition/application facade.

pub mod extension_coordinator;
pub mod extension_install;
pub mod extension_manage;
pub mod extension_runtime_router;
pub mod extension_service;
pub mod extension_snapshot;
pub mod gmail;
pub mod subprocess;

pub use extension_install::ExtensionInstallService;
pub use extension_manage::{
    DenyPermissionElevation, ExplicitOperatorApproval, ExtensionApprovalDecision,
    ExtensionApprovalPort, ExtensionApprovalRequest, ExtensionDoctorResult, ExtensionManageService,
};
pub use extension_service::{
    ActivatedExtensions, ExtensionActivationDecision, ExtensionDecisionSink, ExtensionService,
    NoopExtensionDecisionSink, SessionExtensionPolicy, SpineExtensionDecisionSink,
};
pub use extension_snapshot::{
    ExtensionAgentProfileAsset, ExtensionConnectorAsset, ExtensionRuntimeSnapshot,
    ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
pub use std::path::Path;

/// Consumer-owned extension inspection port (M8.4 HandlerPorts narrowing).
#[async_trait::async_trait]
pub trait ExtensionsPort: Send + Sync {
    fn list(&self) -> anyhow::Result<Vec<corpus::extension::store::InstalledPackageRecord>>;
    fn show(
        &self,
        package_id: &str,
    ) -> anyhow::Result<Vec<corpus::extension::store::InstalledPackageRecord>>;
    fn doctor(&self, package_id: &str) -> anyhow::Result<extension_manage::ExtensionDoctorResult>;
    async fn install(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn enable(
        &self,
        actor: &str,
        package_id: &str,
        approve_permissions: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn disable(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn upgrade(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
        approve_permissions: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn purge(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn rollback(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
    async fn remove(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1>;
}

#[async_trait::async_trait]
impl ExtensionsPort for extension_coordinator::ExtensionCoordinator {
    fn list(&self) -> anyhow::Result<Vec<corpus::extension::store::InstalledPackageRecord>> {
        self.list()
    }
    fn show(
        &self,
        package_id: &str,
    ) -> anyhow::Result<Vec<corpus::extension::store::InstalledPackageRecord>> {
        self.show(package_id)
    }
    fn doctor(&self, package_id: &str) -> anyhow::Result<extension_manage::ExtensionDoctorResult> {
        self.doctor(package_id)
    }
    async fn install(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.install(actor, package_path, trust_workspace).await
    }
    async fn enable(
        &self,
        actor: &str,
        package_id: &str,
        approve_permissions: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.enable(actor, package_id, approve_permissions).await
    }
    async fn disable(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.disable(actor, package_id).await
    }
    async fn upgrade(
        &self,
        actor: &str,
        package_path: &Path,
        trust_workspace: bool,
        approve_permissions: bool,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.upgrade(actor, package_path, trust_workspace, approve_permissions)
            .await
    }
    async fn purge(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.purge(actor, package_id).await
    }
    async fn rollback(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.rollback(actor, package_id).await
    }
    async fn remove(
        &self,
        actor: &str,
        package_id: &str,
    ) -> anyhow::Result<gateway::protocol::extension::ExtensionMutationReceiptV1> {
        self.remove(actor, package_id).await
    }
}
