//! Typed construction unit for the base daemon tool registry.

use std::sync::Arc;

use corpus::tools::tools::{web_search::WebSearchConfig, ToolRegistry};
use fabric::{network_policy::NetworkPolicy, Clock, Registry};
use mnemosyne::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};

use super::memory::MemoryComposition;

pub(super) struct ToolCompositionInput {
    pub(super) network_policy: NetworkPolicy,
    pub(super) search: Option<WebSearchConfig>,
    pub(super) stores: MemoryComposition,
    pub(super) clock: Arc<dyn Clock>,
    /// SQLite path for persisting the task list across daemon restarts.
    /// `None` keeps tasks in-memory only.
    pub(super) tasks_db: Option<std::path::PathBuf>,
    pub(super) agora: Arc<dyn fabric::AgoraService>,
}

pub(super) struct ToolComposition {
    pub(super) registry: ToolRegistry,
    pub(super) stores: MemoryComposition,
}

pub(super) fn compose(input: ToolCompositionInput) -> ToolComposition {
    let mut registry = ToolRegistry::with_network_policy_search_and_tasks(
        input.network_policy,
        input.search,
        input.tasks_db,
    );
    registry
        .bind_agora_task_tools(input.agora, fabric::ProcessId::new())
        .expect("built-in task tools can be rebound to Agora");
    let _ = registry.register(Arc::new(CoreMemoryAppendTool {
        memory: input.stores.core.clone(),
        clock: input.clock.clone(),
    }));
    let _ = registry.register(Arc::new(CoreMemoryReplaceTool {
        memory: input.stores.core.clone(),
        clock: input.clock.clone(),
    }));
    let _ = registry.register(Arc::new(MemorySearchTool {
        recall: input.stores.recall.clone(),
        core_memory: input.stores.core.clone(),
        fact_store: Some(input.stores.facts.clone()),
        clock: input.clock,
    }));

    ToolComposition {
        registry,
        stores: input.stores,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_memory_tools_from_injected_dependencies() {
        let root = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(kernel::chronos::TestClock::new(100, 0));
        let memory = super::super::memory::compose(super::super::memory::MemoryCompositionInput {
            data_dir: root.path(),
            clock: clock.clone(),
        })
        .unwrap();
        let composition = compose(ToolCompositionInput {
            network_policy: NetworkPolicy::default(),
            search: None,
            stores: memory,
            clock,
            tasks_db: None,
            agora: Arc::new(agora::AgoraRegistry::new(Arc::new(
                kernel::chronos::TestClock::default(),
            ))),
        });

        for name in ["core_memory_append", "core_memory_replace", "memory_search"] {
            assert!(composition.registry.get(name).is_some(), "missing {name}");
        }
    }
}
