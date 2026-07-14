//! SystemdHost — runtime host that integrates with systemd service management.
//!
//! Unlike `DaemonHost`, this host:
//! - Sends `sd_notify(READY=1)` after init to signal service readiness.
//! - Spawns a watchdog task that sends `sd_notify(WATCHDOG=1)` periodically.
//! - Handles SIGTERM (delivered by systemd) instead of Ctrl+C for graceful shutdown.
//! - Does NOT manage a PID file (systemd tracks the process via cgroups).
//!
//! Reuses `RuntimeCore::bootstrap`, `UnixServer`, and `McpEmbedded`
//! identically to `DaemonHost`.

use aletheon_kernel::chronos::Timer;
use anyhow::Result;
use fabric::Clock;
use fabric::MonoTime;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::core::runtime_core::RuntimeCore;
use crate::host::load_dotenv;
use crate::r#impl::daemon::mcp_embedded::McpEmbedded;
use crate::r#impl::daemon::server;

/// Send a status message to systemd via the notification socket.
///
/// Reads `$NOTIFY_SOCKET` from the environment. If the variable is unset or
/// the socket is unreachable, the call is silently ignored (this is safe for
/// running outside of systemd, e.g. in development).
fn sd_notify(msg: &str) {
    let socket_path = match std::env::var("NOTIFY_SOCKET") {
        Ok(p) if !p.is_empty() => p,
        _ => return,
    };

    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };

    let _ = sock.send_to(msg.as_bytes(), &socket_path);
}

/// Read `WATCHDOG_USEC` from the environment, returning half the value in
/// microseconds for the watchdog ping interval. Returns `None` if watchdog
/// is not enabled.
fn watchdog_interval_usec() -> Option<u64> {
    let usec: u64 = std::env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if usec == 0 {
        return None;
    }
    Some(usec / 2)
}

/// A deployment host for running under systemd supervision.
pub struct SystemdHost {
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
    enable_evolution: bool,
    core: Option<RuntimeCore>,
    clock: Arc<dyn Clock>,
    started_at: MonoTime,
}

impl SystemdHost {
    pub fn new(
        config_path: Option<PathBuf>,
        env_path: Option<PathBuf>,
        socket: PathBuf,
        enable_evolution: bool,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let started_at = clock.mono_now();
        Self {
            config_path,
            env_path,
            socket,
            enable_evolution,
            core: None,
            clock,
            started_at,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl crate::host::RuntimeHost for SystemdHost {
    async fn init(&mut self) -> Result<()> {
        // ── .env ────────────────────────────────────────────────────
        let env_path = self.env_path.take().unwrap_or_else(|| {
            let path = fabric::paths::env_file();
            if path.exists() {
                return path;
            }
            PathBuf::from(".env")
        });
        load_dotenv(&env_path);

        // ── Runtime core bootstrap ──────────────────────────────────
        let config_path = self.config_path.take();
        self.core = Some(RuntimeCore::bootstrap(config_path, self.enable_evolution).await?);

        // ── Data dir ────────────────────────────────────────────────
        let core = self.core.as_ref().unwrap();
        let data_dir = &core.daemon_config.data_dir;
        tracing::info!(data_dir = %data_dir, "Creating data directory...");
        std::fs::create_dir_all(data_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create data dir '{}': {}", data_dir, e))?;

        // ── Watchdog task ───────────────────────────────────────────
        if let Some(interval_usec) = watchdog_interval_usec() {
            let interval = Duration::from_micros(interval_usec);
            let clock = core.request_handler.subsystems.ports.clock.clone();
            tracing::info!(
                interval_us = interval_usec,
                "Systemd watchdog enabled, pinging every {} us",
                interval_usec
            );
            tokio::spawn(async move {
                loop {
                    Timer::sleep(&*clock, interval).await;
                    sd_notify("WATCHDOG=1");
                }
            });
        }

        // ── Notify systemd: ready ───────────────────────────────────
        sd_notify("READY=1");
        tracing::info!(
            elapsed_ms = %(self.clock.mono_now().0.saturating_sub(self.started_at.0)),
            "SystemdHost ready"
        );

        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        let core = self.core.expect("init() must be called before serve()");
        let request_handler = core.request_handler;
        let cancel_token = core.cancel_token;
        let socket = self.socket;
        let pulse_handle = core.pulse_handle;
        let clock = request_handler.subsystems.ports.clock.clone();

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
        tracing::info!(path = %mcp_socket.display(), "MCP embedded server started");

        // ── Unix server ─────────────────────────────────────────────
        tracing::info!(socket = %socket.display(), "Binding unix socket...");
        let owner_uid = nix::unistd::Uid::current().as_raw();
        let group_gid = nix::unistd::Gid::current().as_raw();
        let mut unix_server = server::UnixServer::new(
            &socket,
            request_handler,
            cancel_token.clone(),
            owner_uid,
            group_gid,
            self.clock.clone(),
        )
        .await?;

        // ── SIGTERM handler (systemd sends SIGTERM for shutdown) ────
        let shutdown_token = cancel_token.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => return,
            };
            term.recv().await;
            tracing::info!("Received SIGTERM, initiating graceful shutdown...");
            shutdown_token.cancel();
        });

        // ── Block until shutdown ────────────────────────────────────
        unix_server.run().await?;

        // ── Graceful shutdown: stop LlmPulse ────────────────────────
        if let Some((shutdown_tx, handle)) = pulse_handle {
            let _ = shutdown_tx.send(true);
            let _ = Timer::timeout(&*clock, Duration::from_secs(2), handle).await;
        }

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        if let Some(ref core) = self.core {
            core.cancel_token.cancel();
        }

        if let Some(ref mut core) = self.core {
            if let Some((shutdown_tx, _handle)) = core.pulse_handle.take() {
                let _ = shutdown_tx.send(true);
            }
        }

        Ok(())
    }
}
