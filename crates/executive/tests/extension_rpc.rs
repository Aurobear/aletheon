use std::sync::Arc;

use executive::application::extension_coordinator::{
    ExtensionCoordinator, ExtensionRuntimePublisher,
};
use executive::application::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
use fabric::protocol::client::ClientRpcRequest;
use fabric::protocol::extension::{ExtensionEnableRequestV1, EXTENSION_PROTOCOL_SCHEMA_V1};
use tempfile::TempDir;

struct NoopPublisher;

#[async_trait::async_trait]
impl ExtensionRuntimePublisher for NoopPublisher {
    async fn probe(&self, _: &ExtensionRuntimeSnapshot) -> anyhow::Result<()> {
        Ok(())
    }

    async fn publish(
        &self,
        _: Arc<ExtensionRuntimeSnapshot>,
        _: Arc<ExtensionRuntimeSnapshot>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn extension_enable_uses_the_versioned_fabric_method_contract() {
    let request = ClientRpcRequest::ExtensionEnable(ExtensionEnableRequestV1 {
        schema_version: EXTENSION_PROTOCOL_SCHEMA_V1,
        package_id: "aurb.core".into(),
        approve_permissions: true,
    })
    .to_json_rpc(Some(7))
    .unwrap();
    assert_eq!(request["method"], "extension.enable");
    assert_eq!(request["params"]["schema_version"], 1);
    assert_eq!(request["params"]["approve_permissions"], true);
    assert!(request["params"].get("actor").is_none());
}

#[test]
fn coordinator_read_rpc_authority_has_no_client_selected_store() {
    let temp = TempDir::new().unwrap();
    let coordinator = ExtensionCoordinator::new(
        temp.path(),
        ExtensionSnapshotCompiler::default(),
        Arc::new(NoopPublisher),
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
    .unwrap();
    assert!(coordinator.list().unwrap().is_empty());
    assert!(coordinator.show("missing").is_err());
}
