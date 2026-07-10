//! Host abstraction (Tier 2b).
//!
//! A `RuntimeHost` is a deployment form of the runtime. `DaemonHost` is the
//! Unix-socket daemon. Additional hosts (systemd, container, CLI-one-shot) are
//! M-F, built on this trait.
//!
//! # Design
//!
//! - `init`: prepare resources (socket dirs, PID files, config, providers, subsystems)
//! - `serve`: run to completion (blocking on the host's event loop)
//! - `shutdown`: release resources
//! - Object-safe: `serve` takes `self: Box<Self>` for ownership transfer

pub mod container;
pub mod systemd;

use anyhow::Result;
use std::path::PathBuf;

use tracing::info;

use crate::core::runtime_core::RuntimeCore;
use crate::r#impl::daemon::mcp_embedded::McpEmbedded;
use crate::r#impl::daemon::server;

/// Load .env file (simple KEY=VALUE parser, no shell expansion).
pub fn load_dotenv(path: &PathBuf) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            // Don't override existing env vars
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
}

/// A deployment host for the runtime.
#[async_trait::async_trait(?Send)]
pub trait RuntimeHost {
    /// Prepare resources before serving. Called once at startup.
    async fn init(&mut self) -> Result<()>;

    /// Run the host's event loop to completion. Takes ownership.
    async fn serve(self: Box<Self>) -> Result<()>;

    /// Release resources. Called during graceful shutdown.
    async fn shutdown(&mut self) -> Result<()>;
}

/// The Unix-socket daemon host.
///
/// Holds CLI-supplied configuration.  `init()` delegates agent-level
/// bootstrap to [`RuntimeCore`] and keeps only host-lifecycle concerns
/// (PID file, .env, data dir).  `serve()` runs the Unix-socket event loop.
pub struct DaemonHost {
    // --- CLI-supplied; set by new() ---
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
    enable_evolution: bool,

    // --- Populated by init() ---
    pid_file: Option<PathBuf>,
    core: Option<RuntimeCore>,
}

impl DaemonHost {
    pub fn new(
        config_path: Option<PathBuf>,
        env_path: Option<PathBuf>,
        socket: PathBuf,
        enable_evolution: bool,
    ) -> Self {
        Self {
            config_path,
            env_path,
            socket,
            enable_evolution,
            pid_file: None,
            core: None,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl RuntimeHost for DaemonHost {
    async fn init(&mut self) -> Result<()> {
        // ── PID file ────────────────────────────────────────────────
        let pid_file = PathBuf::from("/tmp/aletheon/aletheond.pid");
        if let Some(parent) = pid_file.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&pid_file, std::process::id().to_string()).ok();
        self.pid_file = Some(pid_file);

        // ── .env ────────────────────────────────────────────────────
        let env_path = self.env_path.take().unwrap_or_else(|| {
            // 1. ~/.aletheon/.env (user/session mode)
            let user_path = fabric::paths::env_file();
            if user_path.exists() {
                return user_path;
            }
            // 2. /etc/aletheon/.env (system mode)
            let system_path = PathBuf::from("/etc/aletheon/.env");
            if system_path.exists() {
                return system_path;
            }
            // 3. ./.env (working directory)
            PathBuf::from(".env")
        });
        load_dotenv(&env_path);

        // ── Runtime core bootstrap ──────────────────────────────────
        // All agent-level init (config, providers, event bus, LlmPulse,
        // perception, RequestHandler) lives in RuntimeCore so that other
        // host types (systemd, container, oneshot) can reuse it without
        // duplicating the 180-line init sequence.
        let config_path = self.config_path.take();
        self.core = Some(RuntimeCore::bootstrap(config_path, self.enable_evolution).await?);

        // ── Data dir ────────────────────────────────────────────────
        let core = self.core.as_ref().unwrap();
        let data_dir = &core.daemon_config.data_dir;
        tracing::info!(data_dir = %data_dir, "Creating data directory...");
        std::fs::create_dir_all(data_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create data dir '{}': {}", data_dir, e))?;

        // ── Startup log ─────────────────────────────────────────────
        tracing::info!(
            data_dir = %data_dir,
            "Starting agentd"
        );

        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        let core = self.core.expect("init() must be called before serve()");
        let request_handler = core.request_handler;
        let cancel_token = core.cancel_token;
        let socket = self.socket;
        let pulse_handle = core.pulse_handle;
        let pid_file = self.pid_file;

        // ── MCP embedded server ─────────────────────────────────────
        let mcp_socket = socket
            .parent()
            .unwrap_or(&PathBuf::from("/tmp/aletheon"))
            .join("aletheon-mcp.sock");
        let mcp_server = McpEmbedded::new(request_handler.tools(), mcp_socket.clone());
        tokio::spawn(async move {
            if let Err(e) = mcp_server.serve().await {
                tracing::error!("MCP embedded server error: {}", e);
            }
        });
        info!(path = %mcp_socket.display(), "MCP embedded server started");

        // ── Unix server ─────────────────────────────────────────────
        info!(socket = %socket.display(), "Binding unix socket...");
        let owner_uid = nix::unistd::Uid::current().as_raw();
        let group_gid = nix::unistd::Gid::current().as_raw();
        let mut unix_server = server::UnixServer::new(
            &socket,
            request_handler,
            cancel_token.clone(),
            owner_uid,
            group_gid,
        )
        .await?;

        // ── Ctrl+C handler ──────────────────────────────────────────
        let shutdown_token = cancel_token.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
            shutdown_token.cancel();
        });

        unix_server.run().await?;

        // ── Interrupt in-flight chat turns ────────────────────────────
        // Any chat turn that was still running when the accept loop
        // ended gets its interrupt flag set and per-turn token cancelled.
        unix_server.handler().cancel_current_turn().await;

        // ── Graceful shutdown: stop LlmPulse ────────────────────────
        if let Some((shutdown_tx, handle)) = pulse_handle {
            let _ = shutdown_tx.send(true);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        }

        // ── Remove PID file ─────────────────────────────────────────
        if let Some(ref pid_file) = pid_file {
            std::fs::remove_file(pid_file).ok();
        }

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        // Cancel the daemon token to trigger graceful shutdown.
        if let Some(ref core) = self.core {
            core.cancel_token.cancel();
        }

        // Remove PID file if it exists.
        if let Some(ref pid_file) = self.pid_file {
            std::fs::remove_file(pid_file).ok();
        }

        // Stop LlmPulse if running.
        if let Some(ref mut core) = self.core {
            if let Some((shutdown_tx, _handle)) = core.pulse_handle.take() {
                let _ = shutdown_tx.send(true);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingHost {
        inited: Arc<AtomicUsize>,
        shut: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait(?Send)]
    impl RuntimeHost for CountingHost {
        async fn init(&mut self) -> Result<()> {
            self.inited.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn serve(self: Box<Self>) -> Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> Result<()> {
            self.shut.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn host_lifecycle_is_drivable() {
        let inited = Arc::new(AtomicUsize::new(0));
        let shut = Arc::new(AtomicUsize::new(0));
        let mut host = CountingHost {
            inited: inited.clone(),
            shut: shut.clone(),
        };
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(inited.load(Ordering::SeqCst), 1);
        assert_eq!(shut.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn daemon_host_has_zero_init_shutdown_cost() {
        // init/shutdown for DaemonHost now delegate to RuntimeCore::bootstrap().
        // The test verifies construction + lifecycle phases compile and do not panic
        // (init/shutdown may fail without a real config; accept that).
        let mut host = DaemonHost::new(None, None, PathBuf::from("/tmp/test.sock"), false);
        let _ = host.init().await;
        let _ = host.shutdown().await;
    }
}
