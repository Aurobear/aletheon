use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use aletheon_kernel::capability::ToolExecutor;
use async_trait::async_trait;
use corpus::{DefaultCorpusService, ExtensionCatalog, ExtensionGrant};
use executive::core::config::{ConfigSource, ConfigSourceKind};
use executive::service::extension_service::{
    ExtensionActivationDecision, ExtensionDecisionSink, ExtensionService, SessionExtensionPolicy,
    EXTENSION_ACTIVATION_EVENT_V1,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityId, CapabilityRequest, CapabilityResult, CapabilityScope, ExecutionPermit,
    ExtensionDescriptor, ExtensionId, ExtensionKind, PrincipalId,
};

#[derive(Default)]
struct NeverExecutor;
#[async_trait]
impl ToolExecutor for NeverExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        _: &ExecutionPermit,
    ) -> CapabilityResult {
        panic!(
            "discovery and activation must not execute {}",
            request.call.name
        )
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<ExtensionActivationDecision>>);
#[async_trait]
impl ExtensionDecisionSink for RecordingSink {
    async fn record(&self, decision: ExtensionActivationDecision) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(decision);
        Ok(())
    }
}

fn descriptor(kind: ExtensionKind, name: &str, capability: &str) -> ExtensionDescriptor {
    ExtensionDescriptor::new(
        kind,
        name,
        "1",
        name,
        CapabilityId(capability.into()),
        RiskLevel::ReadOnly,
    )
    .unwrap()
}

#[tokio::test]
async fn activation_intersects_config_agent_and_e03_grants_and_records_versioned_decisions() {
    let catalog = ExtensionCatalog::new([
        descriptor(ExtensionKind::Hook, "audit", "hook.audit"),
        descriptor(ExtensionKind::Plugin, "git", "plugin.git"),
        descriptor(ExtensionKind::Mcp, "search", "mcp.search"),
    ])
    .unwrap();
    let corpus = Arc::new(DefaultCorpusService::new(catalog, Arc::new(NeverExecutor)));
    let sink = Arc::new(RecordingSink::default());
    let service = ExtensionService::new(corpus, sink.clone());
    let audit = ExtensionId::new(ExtensionKind::Hook, "audit").unwrap();
    let git = ExtensionId::new(ExtensionKind::Plugin, "git").unwrap();
    let search = ExtensionId::new(ExtensionKind::Mcp, "search").unwrap();
    let grant = ExtensionGrant {
        grant_id: "e03-grant".into(),
        principal: PrincipalId("user:1".into()),
        session_id: "session-1".into(),
        agent_id: None,
        capabilities: vec![
            CapabilityId("hook.audit".into()),
            CapabilityId("plugin.git".into()),
        ],
        resources: CapabilityScope::default(),
    };
    let policy = SessionExtensionPolicy {
        enabled: BTreeSet::from([audit.clone()]),
        agent_name: Some("assistant".into()),
        provenance: BTreeMap::from([(
            audit.as_str().to_string(),
            ConfigSource::new(ConfigSourceKind::Project, "/repo/.aletheon/config.toml"),
        )]),
    };

    let activated = service
        .activate(grant, vec![git, search, audit.clone()], &policy)
        .await
        .unwrap();
    assert_eq!(activated.receipt.extensions, vec![audit]);
    let decisions = sink.0.lock().unwrap();
    assert_eq!(decisions.len(), 3);
    assert!(decisions
        .iter()
        .all(|decision| decision.schema == EXTENSION_ACTIVATION_EVENT_V1));
    assert_eq!(
        decisions
            .iter()
            .filter(|decision| decision.approved)
            .count(),
        1
    );
    assert!(decisions
        .iter()
        .any(|decision| decision.reason.contains("not granted")));
    assert!(decisions
        .iter()
        .any(|decision| decision.reason.contains("disabled")));
    assert!(decisions
        .iter()
        .all(|decision| !serde_json::to_string(decision).unwrap().contains("secret")));
}
