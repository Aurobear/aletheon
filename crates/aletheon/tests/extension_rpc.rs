use std::sync::Arc;

use aletheon::extensions::extension_coordinator::{
    ExtensionCoordinator, ExtensionRuntimePublisher,
};
use aletheon::extensions::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
use gateway::protocol::{Command, ExtensionRequest, EXTENSION_PROTOCOL_SCHEMA_V1};
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
fn extension_enable_uses_the_versioned_gateway_method_contract() {
    let request = Command::ManageExtension(ExtensionRequest::Enable {
        id: "aurb.core".into(),
        approve_permissions: true,
    });
    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(EXTENSION_PROTOCOL_SCHEMA_V1, 1);
    assert_eq!(encoded["ManageExtension"]["Enable"]["id"], "aurb.core");
    assert_eq!(
        encoded["ManageExtension"]["Enable"]["approve_permissions"],
        true
    );
    assert!(encoded["ManageExtension"]["Enable"].get("actor").is_none());
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
