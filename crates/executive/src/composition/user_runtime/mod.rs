//! Per-user execution runtime.
//!
//! The runtime owns user state, protocol handling, approvals, tools, and the
//! private client socket. Model inference is available only through an injected
//! narrow port, normally `CoreRpcClient`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use fabric::paths::UserRuntimePaths;
use kernel::chronos::SystemClock;
use tokio_util::sync::CancellationToken;

use crate::application::inference_port::InferencePort;
use crate::composition::config::ModelRoutingConfig;
use crate::host::daemon::handler::RequestHandler;
use crate::host::daemon::server::{process_inherited_listener, UnixServer};
use crate::host::daemon::DaemonConfig;

pub struct UserRuntimeConfig {
    request: DaemonConfig,
    paths: UserRuntimePaths,
    socket: PathBuf,
    model_routing: ModelRoutingConfig,
    model_aliases: HashMap<String, String>,
    goal_runtime: cognit::config::GoalRuntimeConfig,
    pi_runtime: crate::composition::config::CodingRuntimeConfig,
    grok_hardening: crate::composition::config::GrokHardeningConfig,
    evaluation: crate::composition::config::EvaluationSettings,
    governed_review: crate::composition::config::GovernedReviewSettings,
    sandbox_profiles: fabric::SandboxProfiles,
    network_policy: fabric::network_policy::NetworkPolicy,
    agent_profiles: crate::composition::config::AgentProfilesConfig,
}

impl UserRuntimeConfig {
    pub fn load(
        config_path: Option<&Path>,
        paths: UserRuntimePaths,
        socket: PathBuf,
        enable_evolution: bool,
        enable_execd: bool,
    ) -> anyhow::Result<Self> {
        let loaded = crate::composition::config::load_for_host(None, config_path)?;
        Self::from_loaded(loaded, paths, socket, enable_evolution, enable_execd)
    }

    fn from_loaded(
        loaded: crate::composition::config::LoadedConfig,
        paths: UserRuntimePaths,
        socket: PathBuf,
        enable_evolution: bool,
        enable_execd: bool,
    ) -> anyhow::Result<Self> {
        let integrations = loaded
            .preflight_integrations(&crate::composition::config::EnvironmentCredentialResolver)
            .context("optional integration startup preflight")?;
        let mut app = loaded.value;
        // CLI activation is additive: an absent flag preserves the layered
        // config value, while `--execd` can only enable the backend.
        apply_execd_override(&mut app.grok_hardening, enable_execd);
        app.memory
            .policy
            .validate()
            .context("validating memory judgment policy")?;
        let robot = app
            .resolve_robot_config()
            .context("resolving Robot/Policy configuration")?;
        let crate::composition::config::AppConfig {
            memory: crate::composition::config::MemoryConfig { supplemental, .. },
            ..
        } = &app;
        let mut deployment = app.deployment.clone();
        deployment.mode = cognit::config::DeploymentMode::User;
        deployment.paths.state_root = paths.state_root.clone();
        deployment.paths.state = paths.state_root.join("state");
        deployment.paths.goals = paths.state_root.join("goals");
        deployment.paths.sessions = paths.state_root.join("sessions");
        deployment.paths.mnemosyne = paths.state_root.join("mnemosyne");
        deployment.paths.artifacts = paths.state_root.join("artifacts");
        deployment.paths.worktrees = paths.state_root.join("worktrees");
        deployment.paths.audit = paths.state_root.join("audit");
        deployment.paths.cache_root = paths.cache_root.clone();
        deployment.paths.runtime_root = paths.runtime_root.clone();

        let model = app
            .model_routing
            .default
            .clone()
            .or_else(|| app.agent.default_model.clone())
            .unwrap_or_default();
        let request = DaemonConfig {
            model,
            working_dir: std::env::current_dir()
                .context("resolving user runtime process cwd")?
                .to_string_lossy()
                .into_owned(),
            data_dir: paths.state_root.to_string_lossy().into_owned(),
            system_prompt: app.agent.system_prompt.clone(),
            sandbox_preference: "auto".into(),
            conscious_arbitration_mode: crate::host::daemon::parse_conscious_arbitration_mode(
                app.bootstrap.conscious_arbitration_mode.as_deref(),
            )?,
            enable_evolution,
            evolution_permitted: app.evolution.evolution_permitted,
            evolution_trigger_every_n_turns: app.evolution.trigger_every_n_turns,
            mcp_servers: crate::core::mcp_config::convert_mcp_servers(&app.mcp_servers),
            hooks: app.hooks.clone(),
            telegram: app.telegram.clone(),
            supplemental_memory: supplemental.clone(),
            memory_policy: app.memory_config().clone(),
            deployment,
            backpressure: app.backpressure.clone(),
            agent_admission: app.agent.admission.clone(),
            multi_agent: app.multi_agent.clone(),
            agent_max_iterations: app.agent.max_iterations,
            agent_compaction_threshold_percent: app.agent.compaction_threshold,
            harness_kind: app.agent.harness_kind,
            integrations,
            embodiment_provider: app.integrations.embodiment.clone().unwrap_or_default(),
            robot,
            // RA-03 PR-C gate: default legacy writer.
            session_writer_runtime: false,
        };
        Ok(Self {
            request,
            paths,
            socket,
            model_routing: app.model_routing,
            model_aliases: app.model_aliases,
            goal_runtime: app.goal_runtime.unwrap_or_default(),
            pi_runtime: app.pi_runtime,
            grok_hardening: app.grok_hardening,
            evaluation: app.evaluation,
            governed_review: app.governed_review,
            sandbox_profiles: app.sandbox_profiles,
            network_policy: app.network_policy,
            agent_profiles: app.agent_profiles,
        })
    }

    pub fn fixture() -> Self {
        let root = tempfile_path("aletheon-user-runtime-fixture");
        Self::fixture_at(root)
    }

    pub fn fixture_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let paths = UserRuntimePaths {
            runtime_root: root.join("runtime"),
            state_root: root.join("state"),
            cache_root: root.join("cache"),
        };
        let socket = paths.runtime_root.join("aletheon.sock");
        let loaded = crate::composition::config::merge_layers(std::iter::empty())
            .expect("default user runtime fixture config must load");
        let mut config = Self::from_loaded(loaded, paths, socket, false, false)
            .expect("default user runtime fixture config must load");
        config.request.mcp_servers.clear();
        config.request.telegram.enabled = false;
        config.request.supplemental_memory.enabled = false;

        // Populate the agents directory so that RequestHandler::new can
        // resolve at least one agent profile (required since the profile
        // authority enforcement landed).  Copy the checked-in Markdown
        // definitions; the legacy TOML mirrors are intentionally skipped
        // because the loader only consumes *.md.
        let repo_agents = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        let state_agents = config.paths.state_root.join("agents");
        if !state_agents.exists() {
            std::fs::create_dir_all(&state_agents).expect("create agents dir in fixture");
            if repo_agents.is_dir() {
                for entry in std::fs::read_dir(&repo_agents)
                    .expect("read repo agents dir")
                    .flatten()
                {
                    let src = entry.path();
                    if src
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                    {
                        let dst = state_agents.join(src.file_name().unwrap());
                        std::fs::copy(&src, &dst).expect("copy agent profile into fixture");
                    }
                }
            }
        }

        config
    }

    pub fn paths(&self) -> &UserRuntimePaths {
        &self.paths
    }
}

/// Returns a future that resolves when a process-shutdown signal is received.
///
/// On Unix this listens for both SIGINT (Ctrl+C) and SIGTERM (systemd stop),
/// so that the per-user runtime can execute its graceful shutdown path
/// regardless of how the process is stopped.  On non-Unix platforms only
/// SIGINT (Ctrl+C) is supported.
///
/// Errors (e.g. OS registration failure) are propagated rather than panicking.
#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        res = tokio::signal::ctrl_c() => res,
        opt = sigterm.recv() => {
            match opt {
                Some(()) => Ok(()),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "SIGTERM signal stream closed unexpectedly",
                )),
            }
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

fn apply_execd_override(
    config: &mut crate::composition::config::GrokHardeningConfig,
    cli_enabled: bool,
) {
    config.execd |= cli_enabled;
}

pub struct UserRuntime {
    request_handler: RequestHandler,
    server: Option<UnixServer>,
    paths: UserRuntimePaths,
    cancel: CancellationToken,
}

/// Orchestrate a server future with graceful shutdown via an injected
/// shutdown-signal future.
///
/// Returns `Ok(true)` when the shutdown signal arrived first (the server
/// was cancelled and drained).  Returns `Ok(false)` when the server exited
/// first without cancellation.  Server errors and signal errors are
/// propagated.
async fn shutdown_orchestrator<F>(
    server: F,
    shutdown_signal: impl std::future::Future<Output = std::io::Result<()>>,
    cancel: CancellationToken,
) -> anyhow::Result<bool>
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    tokio::pin!(let server = server;);
    tokio::pin!(let shutdown = shutdown_signal;);

    tokio::select! {
        server_result = &mut server => {
            server_result?;
            Ok(false)
        }
        shutdown_result = &mut shutdown => {
            shutdown_result.context("shutdown signal error")?;
            cancel.cancel();
            server.await?;
            Ok(true)
        }
    }
}

impl UserRuntime {
    pub async fn bootstrap(
        config: UserRuntimeConfig,
        inference: Arc<dyn InferencePort>,
    ) -> anyhow::Result<Self> {
        config.paths.prepare()?;
        let cancel = CancellationToken::new();
        // User runtime follows the same kernel-clock composition rule as the
        // machine runtime: one monotonic epoch shared by handler and server.
        let clock: Arc<dyn fabric::Clock> = Arc::new(SystemClock::new());
        let handler = RequestHandler::new(
            &config.request,
            clock.clone(),
            inference,
            config.model_routing,
            config.model_aliases,
            config.goal_runtime,
            config.pi_runtime,
            config.grok_hardening,
            config.evaluation,
            config.governed_review,
            config.sandbox_profiles.clone(),
            config.network_policy.clone(),
            config.agent_profiles.clone(),
            config.request.enable_evolution,
            None,
            cancel.clone(),
        )
        .await?;
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let server = match process_inherited_listener()? {
            Some(listener) => UnixServer::from_listener(
                listener,
                handler.clone(),
                cancel.clone(),
                uid,
                gid,
                clock.clone(),
            ),
            None => {
                UnixServer::new_user_private(
                    &config.socket,
                    handler.clone(),
                    cancel.clone(),
                    uid,
                    gid,
                    clock,
                )
                .await?
            }
        };
        Ok(Self {
            request_handler: handler,
            server: Some(server),
            paths: config.paths,
            cancel,
        })
    }

    pub async fn health(&self) -> anyhow::Result<()> {
        for path in self.state_paths() {
            if !path.is_dir() {
                anyhow::bail!("user runtime path is unavailable: {}", path.display())
            }
        }
        Ok(())
    }

    pub fn state_paths(&self) -> Vec<PathBuf> {
        vec![
            self.paths.state_root.clone(),
            self.paths.cache_root.clone(),
            self.paths.runtime_root.clone(),
        ]
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut server = self.server.take().context("user server already consumed")?;
        let cancel = self.cancel.clone();

        let server_result =
            shutdown_orchestrator(server.run(), wait_for_shutdown_signal(), cancel).await;

        // Always attempt cleanup regardless of how the server exited.
        self.request_handler.cancel_current_turn().await;
        let cleanup_result = self.request_handler.shutdown_runtime().await;

        match server_result {
            Ok(_) => cleanup_result.context("runtime cleanup after server exit"),
            Err(server_err) => {
                if let Err(e) = &cleanup_result {
                    tracing::warn!(error = %e, "runtime cleanup also failed after server error");
                }
                Err(server_err)
            }
        }
    }
}

fn tempfile_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{label}-{}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::config::GrokHardeningConfig;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn execd_cli_override_is_additive_over_layered_config() {
        let mut off = GrokHardeningConfig::default();
        apply_execd_override(&mut off, false);
        assert!(!off.execd);
        apply_execd_override(&mut off, true);
        assert!(off.execd);

        let mut configured = GrokHardeningConfig {
            execd: true,
            ..Default::default()
        };
        apply_execd_override(&mut configured, false);
        assert!(configured.execd);
    }

    /// Shutdown signal arriving first: token is cancelled, server is drained,
    /// and `was_signalled = true`.
    #[tokio::test]
    async fn shutdown_first_cancels_and_drains_server() {
        let cancel = CancellationToken::new();
        let server = {
            let cancel = cancel.clone();
            async move {
                cancel.cancelled().await;
                Ok(())
            }
        };
        let shutdown = async { Ok::<_, std::io::Error>(()) }; // immediate

        let was_signalled = shutdown_orchestrator(server, shutdown, cancel.clone())
            .await
            .unwrap();
        assert!(was_signalled);
        assert!(cancel.is_cancelled());
    }

    /// Server exiting first: returns without cancellation or leak.
    #[tokio::test]
    async fn server_first_returns_without_cancellation() {
        let cancel = CancellationToken::new();
        let server = async { Ok::<(), anyhow::Error>(()) }; // immediate
        let shutdown = std::future::pending::<std::io::Result<()>>(); // never

        let was_signalled = tokio::time::timeout(
            Duration::from_millis(500),
            shutdown_orchestrator(server, shutdown, cancel.clone()),
        )
        .await
        .expect("must not hang")
        .unwrap();
        assert!(!was_signalled);
        assert!(!cancel.is_cancelled());
    }

    /// Signal error is propagated without panic.
    #[tokio::test]
    async fn signal_error_propagates_without_panic() {
        let cancel = CancellationToken::new();
        let server = std::future::pending::<anyhow::Result<()>>(); // never resolves
        let shutdown = async { Err(std::io::Error::other("signal error")) };

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            shutdown_orchestrator(server, shutdown, cancel),
        )
        .await
        .expect("must not hang");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("signal error"),
            "signal error must propagate"
        );
    }

    /// Server error remains authoritative over a pending shutdown signal.
    #[tokio::test]
    async fn server_error_remains_authoritative() {
        let cancel = CancellationToken::new();
        let server = async { Err(anyhow::anyhow!("server fault")) };
        let shutdown = std::future::pending::<std::io::Result<()>>();

        let result = shutdown_orchestrator(server, shutdown, cancel).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("server fault"),
            "server error must be authoritative"
        );
    }

    /// wait_for_shutdown_signal is reachable on all platforms.
    #[test]
    fn wait_for_shutdown_signal_function_exists() {
        let _ = super::wait_for_shutdown_signal;
    }

    // ── Cleanup semantics tests ──

    /// Simulates the `UserRuntime::run` cleanup pattern so we can prove
    /// cleanup always executes and error precedence is correct without
    /// constructing a full RequestHandler.
    async fn run_with_cleanup<F, G>(
        server: F,
        shutdown_signal: G,
        cancel: CancellationToken,
        cleanup: impl std::future::Future<Output = anyhow::Result<()>>,
    ) -> anyhow::Result<bool>
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
        G: std::future::Future<Output = std::io::Result<()>>,
    {
        let server_result = shutdown_orchestrator(server, shutdown_signal, cancel).await;
        let cleanup_result = cleanup.await;
        match server_result {
            Ok(was_signalled) => {
                cleanup_result?;
                Ok(was_signalled)
            }
            Err(server_err) => {
                // Server error is authoritative; acknowledge but discard
                // the cleanup failure.
                let _ = &cleanup_result;
                Err(server_err)
            }
        }
    }

    /// When the server exits first (successfully), cleanup must still run
    /// and its error must propagate.
    #[tokio::test]
    async fn cleanup_runs_after_server_first_exit() {
        let cancel = CancellationToken::new();
        let server = async { Ok::<(), anyhow::Error>(()) };
        let shutdown = std::future::pending::<std::io::Result<()>>();
        let cleanup = async { Err(anyhow::anyhow!("cleanup fault")) };

        let result = run_with_cleanup(server, shutdown, cancel, cleanup).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("cleanup fault"),
            "cleanup error must propagate when server succeeds"
        );
    }

    /// When the shutdown signal arrives first, cleanup must still run.
    #[tokio::test]
    async fn cleanup_runs_after_signal_first_drain() {
        let cancel = CancellationToken::new();
        let server = {
            let cancel = cancel.clone();
            async move {
                cancel.cancelled().await;
                Ok(())
            }
        };
        let shutdown = async { Ok::<_, std::io::Error>(()) };
        let cleanup = async { Ok::<(), anyhow::Error>(()) };

        let result = run_with_cleanup(server, shutdown, cancel, cleanup).await;
        assert!(result.is_ok());
        assert!(result.unwrap(), "signal arrived first");
    }

    /// Server error must remain authoritative even when cleanup also fails.
    #[tokio::test]
    async fn server_error_hides_cleanup_error() {
        let cancel = CancellationToken::new();
        let server = async { Err(anyhow::anyhow!("server fault")) };
        let shutdown = std::future::pending::<std::io::Result<()>>();
        let cleanup = async { Err(anyhow::anyhow!("cleanup fault")) };

        let result = run_with_cleanup(server, shutdown, cancel, cleanup).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("server fault"),
            "server error must be authoritative, got: {msg}"
        );
        assert!(
            !msg.contains("cleanup fault"),
            "cleanup error must not hide server error, got: {msg}"
        );
    }
}
