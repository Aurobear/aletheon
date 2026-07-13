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
use aletheon_kernel::service::ServicePorts;

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
