//! Memory group — episodic, recall, core, fact-store, auto-memory, and objectives.

use std::sync::Arc;

use tokio::sync::Mutex;

use mnemosyne::episodic::EpisodicMemory;
use mnemosyne::AutoMemory;
use mnemosyne::FactStore;

use crate::r#impl::goal::ObjectiveStore;
use crate::CoreMemory;
use crate::RecallMemory;

pub struct MemoryGroup {
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,
}
