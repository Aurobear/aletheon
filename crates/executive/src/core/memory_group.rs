//! Memory group — episodic, recall, core, fact-store, auto-memory, and objectives.

use std::sync::Arc;

use tokio::sync::Mutex;

use mnemosyne::runtime::EpisodicMemory;
use mnemosyne::MemoryService;

use crate::application::approval::ApprovalRepository;
use crate::application::goal::ObjectiveStore;

pub(crate) struct MemoryGroup {
    pub(crate) episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub(crate) objective_store: Arc<Mutex<ObjectiveStore>>,
    /// Durable protected-operation approvals stored beside Goal state.
    pub(crate) approval_repository: Arc<std::sync::Mutex<ApprovalRepository>>,
    /// Unified facade over the canonical memory stores (docs/arch §11).
    pub(crate) memory_service: Arc<dyn MemoryService>,
    /// Sanitized state of the optional supplemental-memory path.
    pub(crate) supplemental_memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
}
