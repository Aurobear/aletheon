//! Scoped composition of non-tool Corpus extensions.

use std::path::Path;
use std::sync::Arc;

pub(super) struct RuntimeExtensionIndex {
    pub catalog: corpus::ExtensionCatalog,
    pub ids: Vec<fabric::ExtensionId>,
    pub capabilities: Vec<fabric::CapabilityId>,
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
) -> anyhow::Result<Arc<dyn crate::service::extension_service::ExtensionDecisionSink>> {
    let decisions: Arc<dyn crate::service::extension_service::ExtensionDecisionSink> = Arc::new(
        crate::service::extension_service::SpineExtensionDecisionSink::new(Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(state_root.join("extension-events.db"))
                .unwrap_or_else(|_| {
                    crate::r#impl::events::SqliteEventSpine::open(":memory:")
                        .expect("in-memory extension decision spine")
                }),
        )),
    );
    let activation = crate::service::ExtensionService::new(corpus, decisions.clone())
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
            &crate::service::SessionExtensionPolicy::default(),
        )
        .await?;
    tracing::info!(
        count = activation.receipt.extensions.len(),
        "Runtime skills, plugins, and hooks activated through scoped catalog"
    );
    Ok(decisions)
}
