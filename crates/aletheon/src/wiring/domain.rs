//! Binary-owned domain service handles used while assembling the daemon.
//!
//! This is deliberately a wiring-only aggregate.  Aletheon owns the
//! application ports and implementations; the `aletheon` composition root
//! owns the decision to bind the concrete services for one daemon instance.

use agora::AgoraService;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::socket_approval::PendingApproval;
use corpus::security::storm_breaker::StormBreaker;
use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;
use mnemosyne::runtime::EpisodicMemory;
use mnemosyne::MemoryService;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Binary-owned composition groups. These handles used to live under
/// `executive::core`; keeping them at the composition root prevents the
/// application crate from exposing a second public component graph.
pub(crate) struct MemoryGroup {
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub objective_store: Arc<Mutex<crate::wiring::application::goal::ObjectiveStore>>,
    pub approval_repository:
        Arc<std::sync::Mutex<adapters_sqlite::approval_repository::ApprovalRepository>>,
    pub memory_service: Arc<dyn MemoryService>,
    pub local_memory_service: Arc<dyn MemoryService>,
    pub supplemental_memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
    pub supplemental_spool: Option<Arc<mnemosyne::supplemental::SupplementalSpool>>,
}

pub(crate) type ToolRunnerHandle = Arc<Mutex<ToolRunnerWithGuard>>;
pub(crate) struct SecurityGroup {
    pub tool_runner: ToolRunnerHandle,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub pending_approvals: crate::wiring::application::admin_service::PendingApprovals,
    pub session_approvals: crate::wiring::application::admin_service::ScopedApprovalCache,
}

pub(crate) type ToolRegistryHandle = Arc<Mutex<ToolRegistry>>;
pub(crate) type HookRegistryHandle = Arc<Mutex<HookRegistry>>;
pub(crate) struct CorpusGroup {
    pub tools: ToolRegistryHandle,
    pub hook_registry: HookRegistryHandle,
}

pub(crate) struct SessionGroup {
    pub default_session_id: Arc<Mutex<String>>,
    pub session_created_at: Arc<Mutex<HashMap<String, ::contracts::MonoTime>>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub context_window: usize,
    pub data_dir: PathBuf,
}

#[derive(Clone)]
pub(crate) struct DomainServices {
    agora: Arc<dyn AgoraService>,
    metacog: Arc<dyn metacog::MetacogService>,
    corpus: Arc<dyn corpus::CorpusService>,
    cognition: Arc<dyn cognit::harness::CognitiveSessionFactory>,
}

impl DomainServices {
    pub(crate) fn new(
        agora: Arc<dyn AgoraService>,
        metacog: Arc<dyn metacog::MetacogService>,
        corpus: Arc<dyn corpus::CorpusService>,
        cognition: Arc<dyn cognit::harness::CognitiveSessionFactory>,
    ) -> Self {
        Self {
            agora,
            metacog,
            corpus,
            cognition,
        }
    }

    pub(crate) fn agora(&self) -> Arc<dyn AgoraService> {
        self.agora.clone()
    }

    pub(crate) fn metacog(&self) -> Arc<dyn metacog::MetacogService> {
        self.metacog.clone()
    }

    pub(crate) fn corpus(&self) -> Arc<dyn corpus::CorpusService> {
        self.corpus.clone()
    }

    pub(crate) fn cognition(&self) -> Arc<dyn cognit::harness::CognitiveSessionFactory> {
        self.cognition.clone()
    }
}
