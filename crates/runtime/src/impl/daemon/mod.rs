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
//! - `session_manager.rs` — conversation history, journaling, compaction
//!
//! Business logic lives in `crate::core/` (orchestrator, session_gateway)
//! and in subsystem crates (cognit, dasein, corpus, memory, metacog).

pub mod cache_shape;
pub mod debug_handler;
pub mod handler;
pub mod mcp_embedded;
pub mod model_router;
pub mod prefix_builder;
pub mod server;
pub mod session_manager;

use std::path::PathBuf;

use anyhow::Result;

use crate::host::RuntimeHost;

/// Daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub sandbox_preference: String,
    /// Enable self-evolution loop (HIGH-risk autonomy — OFF by default).
    pub enable_evolution: bool,
    /// MCP server definitions loaded from config (passed through to McpManager at handler init).
    pub mcp_servers: Vec<corpus::tools::mcp::config::McpServerConfig>,
    /// Hook script configuration from the `hooks` config section.
    pub hooks: crate::core::config::HooksConfig,
}

/// Default config file search paths.
#[allow(dead_code)]
fn default_config_path() -> PathBuf {
    // 1. ~/.aletheon/config.toml
    let path = base::paths::config_file();
    if path.exists() {
        return path;
    }
    // 2. /etc/agentd/config.toml
    PathBuf::from("/etc/agentd/config.toml")
}

/// Run the daemon with the given CLI arguments.
///
/// This is a compatibility wrapper that delegates to `DaemonHost` lifecycle
/// phases. New callers should use `DaemonHost::new()` + `init()` + `serve()`
/// directly. This function remains for backward compatibility with external
/// callers that do not use the `RuntimeHost` trait.
pub async fn run(
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
) -> Result<()> {
    let mut host = crate::host::DaemonHost::new(config_path, env_path, socket, false);
    host.init().await?;
    Box::new(host).serve().await
}
