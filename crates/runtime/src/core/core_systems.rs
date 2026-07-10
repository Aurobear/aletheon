//! CoreSystems — concrete subsystem type bundle.
//!
//! Holds all subsystem types that RequestHandler currently owns directly.
//! During Group B, each field transitions to `Arc<dyn TraitOps>` from
//! base::ops as each subsystem gets its trait implementation migrated.
//!
//! This is the intermediate step between the God Object and the final
//! trait-based architecture defined in RFC-010~013.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use base::kernel::debug_bus::PerfCounter;
use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use memory::episodic::EpisodicMemory;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};

use crate::core::config::HooksConfig;
use crate::core::orchestrator::AletheonRuntime;
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;
use crate::CoreMemory;
use crate::RecallMemory;

use crate::r#impl::daemon::debug_handler::DebugHandler;
use cognit::core::reflector::Reflector;

/// Bundle of subsystem types.
///
/// In Group B, each field transitions to `Arc<dyn TraitOps>`.
pub struct CoreSystems {
    // --- Orchestrator ---
    pub runtime: Arc<Mutex<AletheonRuntime>>,

    // --- Dasein (SelfField) ---
    pub self_field: Arc<Mutex<SelfField>>,

    // --- Mnemosyne (Memory) ---
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,

    // --- Cognit ---
    pub reflector: Reflector,

    // --- Corpus ---
    pub tools: Arc<Mutex<ToolRegistry>>,
    pub tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: Arc<Mutex<HookRegistry>>,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub hooks_config: HooksConfig,

    // --- Metacog ---
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,

    // --- Approval / Security ---
    pub approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub session_approvals: Arc<Mutex<HashMap<String, bool>>>,

    // --- Debug / Observability ---
    pub debug_handler: Arc<DebugHandler>,
    pub debug_perf: Arc<PerfCounter>,
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,

    // --- Prefix / Context ---
    pub cached_prefix: Arc<Mutex<String>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub config_prompt: String,

    // --- Session management ---
    pub default_session_id: Arc<tokio::sync::Mutex<String>>,
    pub session_created_at: Arc<Mutex<HashMap<String, Instant>>>,
    pub data_dir: PathBuf,
    pub context_window: usize,
}
