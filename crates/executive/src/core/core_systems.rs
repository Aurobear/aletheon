//! CoreSystems — concrete subsystem type bundle.
//!
//! Holds all subsystem types that RequestHandler currently owns directly.
//! Subsystem contracts live in `fabric::include` (`CognitOps`, `SelfFieldOps`,
//! `MemoryBackend`, `BodyRuntime`, `AgoraOps`, …); slimming these concrete fields
//! toward `Arc<dyn …>` boundaries is tracked as RFC-018 D5.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use dasein::SelfField;
use fabric::kernel::debug_bus::PerfCounter;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};

use crate::core::corpus_group::CorpusGroup;
use crate::core::memory_group::MemoryGroup;
use crate::core::orchestrator::AletheonExecutive;
use crate::core::security_group::SecurityGroup;
use crate::core::session_group::SessionGroup;
use aletheon_kernel::KernelRuntime;

use crate::r#impl::daemon::debug_handler::DebugHandler;
use cognit::core::reflector::Reflector;

/// Bundle of subsystem types.
///
/// In Group B, each field transitions to `Arc<dyn TraitOps>`.
/// Kernel mechanisms and cognitive domain ports are separate authorities.
pub struct CoreSystems {
    /// Opaque authority for process, operation, space and resource lifecycle.
    pub kernel: Arc<KernelRuntime>,

    /// Executive-owned cognitive domain composition; never stored in Kernel.
    pub domains: crate::core::DomainPorts,

    // --- Orchestrator ---
    pub runtime: Arc<Mutex<AletheonExecutive>>,

    // --- Dasein (SelfField) ---
    pub self_field: Arc<Mutex<SelfField>>,

    // --- Cognit ---
    pub reflector: Reflector,

    // --- Memory Group ---
    pub memory: MemoryGroup,

    // --- Security Group ---
    pub security: SecurityGroup,

    // --- Corpus Group ---
    pub corpus: CorpusGroup,

    // --- Session Group ---
    pub session: SessionGroup,

    // --- Metacog ---
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,

    // --- Debug / Observability ---
    pub debug_handler: Arc<DebugHandler>,
    pub debug_perf: Arc<PerfCounter>,
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,

    /// Shared main-agent process id, written by DaemonTurnOrchestrator's
    /// ensure_main_agent and read by tools (e.g. AgentTool) that need the
    /// process parent for space forking.
    pub main_agent_process_id: Arc<tokio::sync::Mutex<Option<fabric::ProcessId>>>,
}
