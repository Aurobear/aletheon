//! ContainerHost — runs the runtime agent inside a Docker/Podman container.
//!
//! `ContainerHost` manages an agent container lifecycle:
//! - `init()`: loads .env, bootstraps RuntimeCore, verifies container CLI availability.
//! - `serve()`: builds a `docker/podman run` command, streams logs, blocks until exit.
//! - `shutdown()`: sends SIGTERM to the container, waits for graceful exit.
//!
//! The host reuses [`RuntimeCore::bootstrap`] for agent initialization. The
//! UnixServer and McpEmbedded are NOT started inside the ContainerHost —
//! they run *inside* the container (the container entrypoint starts
//! `aletheond` which handles that).

use aletheon_kernel::chronos::Timer;
use anyhow::Result;
use std::path::PathBuf;

use crate::core::runtime_core::RuntimeCore;
use crate::host::load_dotenv;

/// Verify that the given container runtime CLI is available on the system.
fn check_container_cli(cmd: &str) -> Result<()> {
    let output = std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map_err(|_| {
            anyhow::anyhow!(
                "Container runtime '{}' not found. Is docker or podman installed?",
                cmd
            )
        })?;
    if !output.status.success() {
        anyhow::bail!("Container runtime '{}' returned non-zero exit code", cmd);
    }
    let version = String::from_utf8_lossy(&output.stdout);
    tracing::info!(runtime = cmd, version = %version.trim(), "Container runtime detected");
    Ok(())
}

/// A deployment host for running inside a Docker/Podman container.
pub struct ContainerHost {
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    /// Container runtime command: "docker" or "podman".
    runtime_cmd: String,
    /// Container image name.
    image: String,
    enable_evolution: bool,
    core: Option<RuntimeCore>,
}

impl ContainerHost {
    pub fn new(
        config_path: Option<PathBuf>,
        env_path: Option<PathBuf>,
        runtime_cmd: String,
        image: String,
        enable_evolution: bool,
    ) -> Self {
        Self {
            config_path,
            env_path,
            runtime_cmd,
            image,
            enable_evolution,
            core: None,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl crate::host::RuntimeHost for ContainerHost {
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

        // ── Verify container runtime CLI ────────────────────────────
        check_container_cli(&self.runtime_cmd)?;

        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        let core = self.core.expect("init() must be called before serve()");
        let data_dir = core.daemon_config.data_dir.clone();
        let cancel_token = core.cancel_token.clone();
        let pulse_handle = core.pulse_handle;
        let clock = core.request_handler.subsystems.ports.clock.clone();

        // ── Build container run command ─────────────────────────────
        // Mount the data directory so the agent inside the container
        // can persist state to the host filesystem.
        let mut cmd = tokio::process::Command::new(&self.runtime_cmd);
        cmd.arg("run")
            .arg("--rm")
            .arg("--name")
            .arg("aletheon-container")
            .arg("-v")
            .arg(format!("{}:{}", data_dir, data_dir))
            // Pass through the agent data dir env var
            .arg("-e")
            .arg(format!("AGENT_DATA_DIR={}", data_dir));

        // Forward relevant environment variables.
        for (key, val) in std::env::vars() {
            if key.starts_with("AGENT_") || key.starts_with("LLM_") || key == "RUST_LOG" {
                cmd.arg("-e").arg(format!("{}={}", key, val));
            }
        }

        cmd.arg(&self.image);

        // The container entrypoint should start aletheond or aletheon-systemd.
        tracing::info!(
            runtime = %self.runtime_cmd,
            image = %self.image,
            data_dir = %data_dir,
            "Starting container..."
        );

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start container '{}': {}", self.image, e))?;

        // ── Stream container output ─────────────────────────────────
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(stdout) = stdout {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::info!(target: "container", "{}", line);
                }
            });
        }

        if let Some(stderr) = stderr {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: "container", "{}", line);
                }
            });
        }

        // ── Block until container exits or cancel_token fires ───────
        let exit_status = tokio::select! {
            status = child.wait() => {
                status.map_err(|e| anyhow::anyhow!("Container wait error: {}", e))?
            }
            _ = cancel_token.cancelled() => {
                tracing::info!("Shutdown signal received, stopping container...");
                // Send SIGTERM to the container.
                let pid = child.id().unwrap_or_default();
                if pid > 0 {
                    let _ = tokio::process::Command::new(&self.runtime_cmd)
                        .arg("stop")
                        .arg("--time=10")
                        .arg("aletheon-container")
                        .status()
                        .await;
                }
                // Wait for graceful exit.
                let _ = Timer::timeout(
                    &*clock,
                    std::time::Duration::from_secs(15),
                    child.wait(),
                ).await;
                // Force kill if still running.
                let _ = child.kill().await;
                return Ok(());
            }
        };

        tracing::info!(
            exit_code = %exit_status,
            "Container exited"
        );

        // ── Stop LlmPulse (if any spawned inside this host; container
        //    runs its own, but clean up just in case.) ────────────────
        if let Some((shutdown_tx, handle)) = pulse_handle {
            let _ = shutdown_tx.send(true);
            let _ = Timer::timeout(&*clock, std::time::Duration::from_secs(2), handle).await;
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
