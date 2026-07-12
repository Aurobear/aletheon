//! CoreSystems — concrete subsystem type bundle.
//!
//! Holds all subsystem types that RequestHandler currently owns directly.
//! Subsystem contracts live in `fabric::include` (`CognitOps`, `SelfFieldOps`,
//! `MemoryBackend`, `BodyRuntime`, `AgoraOps`, …); slimming these concrete fields
//! toward `Arc<dyn …>` boundaries is tracked as RFC-018 D5.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use fabric::kernel::debug_bus::PerfCounter;
use fabric::AdmissionController;
use fabric::AgoraOps;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use mnemosyne::episodic::EpisodicMemory;

use crate::core::config::HooksConfig;
use crate::core::orchestrator::AletheonExecutive;
use crate::r#impl::goal::ObjectiveStore;
use crate::CoreMemory;
use crate::RecallMemory;
use aletheon_kernel::service_ports::ServicePorts;
use corpus::security::storm_breaker::StormBreaker;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;
use mnemosyne::AutoMemory;
use mnemosyne::FactStore;

use crate::r#impl::daemon::debug_handler::DebugHandler;
use cognit::core::reflector::Reflector;

/// Bundle of subsystem types.
///
/// In Group B, each field transitions to `Arc<dyn TraitOps>`.
/// New code should prefer `ports: ServicePorts` for kernel primitives;
/// domain-specific services (memory, corpus, etc.) remain in `CoreSystems`
/// until their port structs are defined.
pub struct CoreSystems {
    /// Kernel service ports — the canonical access point for process/operation/
    /// supervision/clock/mailbox/admission/agora/budget/lease primitives.
    ///
    /// This field is the first step of Phase 6A contraction. Over time, more
    /// services will migrate from `CoreSystems` into `ServicePorts`.
    pub ports: ServicePorts,

    // --- Orchestrator ---
    pub runtime: Arc<Mutex<AletheonExecutive>>,

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

    /// Shared cognitive workspace (RFC-014). Session-isolated working memory.
    /// Held as `dyn AgoraOps` (RFC-018 issue #3): the first `CoreSystems` field
    /// behind a trait object, so it can be swapped/mocked without the concrete
    /// `AgoraRegistry`.
    pub agora: Arc<dyn AgoraOps>,

    /// Admission controller for capability gating (Phase 5A).
    /// All side-effecting tool invocations route through this controller
    /// via `admit() → execute → settle()`.
    pub admission: Arc<dyn AdmissionController>,

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
