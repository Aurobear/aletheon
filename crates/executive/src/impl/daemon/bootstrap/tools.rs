//! Typed construction unit for the base daemon tool registry.

use std::sync::Arc;

use corpus::tools::tools::{web_search::WebSearchConfig, ToolRegistry};
use fabric::{network_policy::NetworkPolicy, Clock, Registry};
use mnemosyne::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};

use super::memory::MemoryComposition;

pub(super) struct ToolCompositionInput {
    pub(super) network_policy: NetworkPolicy,
    pub(super) search: Option<WebSearchConfig>,
    pub(super) memory: MemoryComposition,
    pub(super) clock: Arc<dyn Clock>,
}

pub(super) struct ToolComposition {
    pub(super) registry: ToolRegistry,
    pub(super) memory: MemoryComposition,
}

pub(super) fn compose(input: ToolCompositionInput) -> ToolComposition {
    let mut registry =
        ToolRegistry::with_network_policy_and_search(input.network_policy, input.search);
    let _ = registry.register(Arc::new(CoreMemoryAppendTool {
        memory: input.memory.core.clone(),
        clock: input.clock.clone(),
    }));
    let _ = registry.register(Arc::new(CoreMemoryReplaceTool {
        memory: input.memory.core.clone(),
        clock: input.clock.clone(),
    }));
    let _ = registry.register(Arc::new(MemorySearchTool {
        recall: input.memory.recall.clone(),
        core_memory: input.memory.core.clone(),
        fact_store: Some(input.memory.facts.clone()),
        clock: input.clock,
    }));

    ToolComposition {
        registry,
        memory: input.memory,
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
            memory,
            clock,
        });

        for name in ["core_memory_append", "core_memory_replace", "memory_search"] {
            assert!(composition.registry.get(name).is_some(), "missing {name}");
        }
    }
}
