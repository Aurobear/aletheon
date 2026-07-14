//! Memory group — episodic, recall, core, fact-store, auto-memory, and objectives.

use std::sync::Arc;

use tokio::sync::Mutex;

use mnemosyne::episodic::EpisodicMemory;
use mnemosyne::AutoMemory;
use mnemosyne::FactStore;
use mnemosyne::MemoryService;

use crate::r#impl::goal::ObjectiveStore;
use mnemosyne::CoreMemory;
use mnemosyne::RecallMemory;

pub struct MemoryGroup {
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,
    /// Unified facade over the 6 memory objects above (docs/arch §11).
    /// Built from the same `Arc<Mutex<_>>` handles; additive, does not
    /// replace direct field access elsewhere.
    pub memory_service: Arc<dyn MemoryService>,
    /// Optional gbrain MCP manager handle for shared memory recall.
    /// `None` when gbrain is disabled or connection failed at startup.
    pub gbrain: Option<Arc<corpus::tools::mcp::manager::McpManager>>,
    /// gbrain runtime configuration (available even when handle is None).
    pub gbrain_config: cognit::config::GbrainMemoryConfig,
}
