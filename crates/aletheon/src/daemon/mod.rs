//! Daemon layer — Gateway (transport/IO) and session management.
//!
//! ## Gateway (transport)
//! - `server.rs` — Unix socket listener, connection accept loop
//! - `handler/` — JSON-RPC dispatch (RequestHandler, chat, rpc methods)
//! - `mcp_embedded.rs` — embedded MCP protocol server
//! - `prefix_builder.rs` — cache-stable system prompt construction
//! - `debug_handler.rs` — debug.* JSON-RPC namespace
//! - `model_router.rs` — per-task-type model selection
//! - `cache_shape.rs` — cache invalidation tracking
//!
//! ## Session management (core-adjacent)
//! - `context_working_set.rs` — conversation history, journaling, compaction
//!
//! Business logic lives in `crate::core/` (orchestrator, session_gateway)
//! and in subsystem crates (cognit, dasein, corpus, memory, metacog).

pub mod context_working_set;
pub mod debug_handler;
pub mod handler;
pub mod legacy_session;
pub mod mcp_embedded;
pub mod model_router;
pub mod server;
pub mod session_projection;
pub mod turn;
pub mod turn_approval_projection;
pub mod turn_engine;
pub mod turn_event_projection;
pub mod turn_lifecycle_adapter;

pub use turn::DaemonTurnOrchestrator;

/// Binary-owned daemon configuration consumed by the local bootstrap.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub model: String,
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub sandbox_preference: String,
    pub conscious_arbitration_mode: ::contracts::ConsciousArbitrationMode,
    pub enable_evolution: bool,
    pub evolution_permitted: bool,
    pub evolution_trigger_every_n_turns: usize,
    pub mcp_servers: Vec<corpus::tools::mcp::config::McpServerConfig>,
    pub hooks: crate::config::HooksConfig,
    pub telegram: crate::config::TelegramChannelConfig,
    pub supplemental_memory: mnemosyne::supplemental_memory::SupplementalMemoryConfig,
    pub memory_policy: mnemosyne::supplemental_memory::MemoryConfig,
    pub deployment: cognit::config::DeploymentConfig,
    pub backpressure: crate::config::BackpressureConfig,
    pub agent_admission: cognit::config::AgentAdmissionConfig,
    pub multi_agent: crate::config::MultiAgentConfig,
    pub agent_max_iterations: usize,
    pub agent_compaction_threshold_percent: usize,
    pub harness_kind: cognit::harness::HarnessKind,
    pub integrations: crate::config::ResolvedIntegrations,
    pub embodiment_provider: crate::config::EmbodimentProviderConfig,
    pub robot: Option<crate::config::ResolvedRobotIntegrationConfig>,
    pub session_writer: crate::config::SessionWriterMode,
}

pub fn parse_conscious_arbitration_mode(
    value: Option<&str>,
) -> anyhow::Result<::contracts::ConsciousArbitrationMode> {
    match value {
        None => Ok(::contracts::ConsciousArbitrationMode::Observe),
        Some("observe") => Ok(::contracts::ConsciousArbitrationMode::Observe),
        Some("enforce") => Ok(::contracts::ConsciousArbitrationMode::Enforce),
        Some(_) => Err(anyhow::anyhow!(
            "ALETHEON_CONSCIOUS_ARBITRATION_MODE must be 'observe' or 'enforce'"
        )),
    }
}

pub mod turn_use_cases;
