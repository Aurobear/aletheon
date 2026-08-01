//! Typed construction unit for daemon memory stores.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use fabric::Clock;
use mnemosyne::runtime::{CoreMemory, FactStore, RecallMemory};
use tokio::sync::Mutex;

use crate::application::memory_gateway::MemoryGatewayService;
use crate::application::memory_maintenance::{
    AgentControlMemorySemanticProposal, MemoryMaintenanceController,
};
use crate::composition::config::MemoryConfig;
use crate::core::MemoryGroup;

pub(super) struct MemoryCompositionInput<'a> {
    pub(super) data_dir: &'a Path,
    pub(super) clock: Arc<dyn Clock>,
}

pub(super) struct MemoryComposition {
    pub(super) core: Arc<Mutex<CoreMemory>>,
    pub(super) recall: Arc<Mutex<RecallMemory>>,
    pub(super) facts: Arc<Mutex<FactStore>>,
}

pub(super) fn compose(input: MemoryCompositionInput<'_>) -> anyhow::Result<MemoryComposition> {
    let core_path = input.data_dir.join("core_memory.json");
    let core = Arc::new(Mutex::new(CoreMemory::load_or_default(&core_path)));
    let recall = Arc::new(Mutex::new(RecallMemory::new(
        &input.data_dir.join("recall_memory.db"),
        input.clock,
    )?));
    let fact_root = input.data_dir.join("mnemosyne");
    std::fs::create_dir_all(&fact_root)
        .with_context(|| format!("creating fact store root: {}", fact_root.display()))?;
    let facts = Arc::new(Mutex::new(
        FactStore::open(&fact_root.join("fact_store.db")).context("opening fact store")?,
    ));

    Ok(MemoryComposition {
        core,
        recall,
        facts,
    })
}

pub(super) fn compose_gateway(
    data_dir: &Path,
    memory: &MemoryGroup,
    clock: Arc<dyn Clock>,
    mcp: Option<Arc<corpus::tools::mcp::manager::McpManager>>,
    config: &MemoryConfig,
) -> anyhow::Result<Arc<MemoryGatewayService>> {
    let mut gateway =
        MemoryGatewayService::open(data_dir, memory.local_memory_service.clone(), clock)
            .context("opening versioned memory gateway")?;
    if let Some(manager) = mcp {
        let supplemental_router = Arc::new(
            crate::adapters::gbrain::McpSupplementalBindingNegotiator::new(
                manager,
                std::time::Duration::from_millis(config.supplemental.request_timeout_ms),
                &config.supplemental.destination_attestations,
            )
            .context("validating supplemental destination attestations")?,
        );
        gateway = gateway
            .with_binding_negotiator(supplemental_router.clone())
            .with_supplemental_recall(supplemental_router);
    }
    Ok(Arc::new(gateway))
}

pub(super) fn compose_maintenance(
    gateway: &Arc<MemoryGatewayService>,
    memory: &MemoryGroup,
    clock: Arc<dyn Clock>,
    control: Arc<dyn fabric::AgentControlPort>,
    config: &MemoryConfig,
) -> anyhow::Result<Arc<MemoryMaintenanceController>> {
    let semantic = Arc::new(
        AgentControlMemorySemanticProposal::new(control, config.policy.clone())
            .context("constructing AgentRuntime memory semantic proposer")?,
    );
    Ok(Arc::new(
        MemoryMaintenanceController::new(
            gateway.intake_ledger(),
            memory.local_memory_service.clone(),
            clock,
            config.policy.clone(),
            semantic,
        )
        .context("constructing memory maintenance controller")?
        .with_projection(
            gateway.binding_registry(),
            memory.supplemental_spool.clone(),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_memory_stores_under_injected_data_root() {
        let root = tempfile::tempdir().unwrap();
        let composition = compose(MemoryCompositionInput {
            data_dir: root.path(),
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
        })
        .unwrap();

        assert!(root.path().join("recall_memory.db").exists());
        assert!(root.path().join("mnemosyne/fact_store.db").exists());
        drop(composition);
    }

    #[test]
    fn reports_fact_root_construction_failure() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("mnemosyne"), "occupied").unwrap();

        assert!(compose(MemoryCompositionInput {
            data_dir: root.path(),
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
        })
        .is_err());
    }
}
