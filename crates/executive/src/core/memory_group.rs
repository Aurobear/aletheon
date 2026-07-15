//! Memory group — episodic, recall, core, fact-store, auto-memory, and objectives.

use std::sync::Arc;

use tokio::sync::Mutex;

use mnemosyne::episodic::EpisodicMemory;
use mnemosyne::AutoMemory;
use mnemosyne::MemoryService;

use crate::r#impl::approval::ApprovalRepository;
use crate::r#impl::goal::ObjectiveStore;
use mnemosyne::RecallMemory;

pub(crate) struct MemoryGroup {
    pub(crate) episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub(crate) recall_memory: Arc<Mutex<RecallMemory>>,
    pub(crate) auto_memory: Arc<Mutex<AutoMemory>>,
    pub(crate) objective_store: Arc<Mutex<ObjectiveStore>>,
    /// Durable protected-operation approvals stored beside Goal state.
    pub(crate) approval_repository: Arc<std::sync::Mutex<ApprovalRepository>>,
    /// Unified facade over the 6 memory objects above (docs/arch §11).
    /// Built from the same `Arc<Mutex<_>>` handles; additive, does not
    /// replace direct field access elsewhere.
    pub(crate) memory_service: Arc<dyn MemoryService>,
    /// Sanitized state of the optional supplemental-memory path.
    pub(crate) supplemental_memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
}
