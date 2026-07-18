//! Per-user execution runtime.
//!
//! The runtime owns user state, protocol handling, approvals, tools, and the
//! private client socket. Model inference is available only through an injected
//! narrow port, normally `CoreRpcClient`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use fabric::paths::UserRuntimePaths;
use tokio_util::sync::CancellationToken;

use crate::core::config::ModelRoutingConfig;
use crate::r#impl::daemon::handler::RequestHandler;
use crate::r#impl::daemon::server::{process_inherited_listener, UnixServer};
use crate::r#impl::daemon::DaemonConfig;
use crate::service::inference_port::InferencePort;

pub struct UserRuntimeConfig {
    request: DaemonConfig,
    paths: UserRuntimePaths,
    socket: PathBuf,
    model_routing: ModelRoutingConfig,
    model_aliases: HashMap<String, String>,
    goal_runtime: cognit::config::GoalRuntimeConfig,
    pi_runtime: cognit::config::PiRuntimeConfig,
    grok_hardening: crate::core::config::GrokHardeningConfig,
    sandbox_profiles: fabric::SandboxProfiles,
}

impl UserRuntimeConfig {
    pub fn load(
        config_path: Option<&Path>,
        paths: UserRuntimePaths,
        socket: PathBuf,
        enable_evolution: bool,
        enable_exec_server: bool,
    ) -> anyhow::Result<Self> {
        let mut app = crate::core::config::load_for_host(None, config_path)?.value;
        // CLI activation is additive: an absent flag preserves the layered
        // config value, while `--exec-server` can only enable the backend.
        apply_exec_server_override(&mut app.grok_hardening, enable_exec_server);
        let crate::core::config::AppConfig {
            memory: crate::core::config::MemoryConfig { gbrain, .. },
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
            conscious_arbitration_mode: crate::r#impl::daemon::conscious_arbitration_mode_from_env(
            )?,
            enable_evolution,
            mcp_servers: crate::core::mcp_config::convert_mcp_servers(&app.mcp_servers),
            hooks: app.hooks.clone(),
            telegram: app.telegram.clone(),
            gbrain_memory: gbrain.clone(),
            deployment,
            backpressure: app.backpressure.clone(),
            agent_admission: app.agent.admission.clone(),
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
            sandbox_profiles: app.sandbox_profiles,
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
        let mut config = Self::load(None, paths, socket, false, false)
            .expect("default user runtime fixture config must load");
        config.request.mcp_servers.clear();
        config.request.telegram.enabled = false;
        config.request.gbrain_memory.enabled = false;
        config
    }

    pub fn paths(&self) -> &UserRuntimePaths {
        &self.paths
    }
}

fn apply_exec_server_override(
    config: &mut crate::core::config::GrokHardeningConfig,
    cli_enabled: bool,
) {
    config.exec_server |= cli_enabled;
}

pub struct UserRuntime {
    request_handler: RequestHandler,
    server: Option<UnixServer>,
    paths: UserRuntimePaths,
    cancel: CancellationToken,
}

impl UserRuntime {
    pub async fn bootstrap(
        config: UserRuntimeConfig,
        inference: Arc<dyn InferencePort>,
    ) -> anyhow::Result<Self> {
        config.paths.prepare()?;
        let cancel = CancellationToken::new();
        let handler = RequestHandler::new(
            &config.request,
            inference,
            config.model_routing,
            config.model_aliases,
            config.goal_runtime,
            config.pi_runtime,
            config.grok_hardening,
            config.sandbox_profiles.clone(),
            config.request.enable_evolution,
            None,
            cancel.clone(),
        )
        .await?;
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let clock = handler.clock();
        let server = match process_inherited_listener()? {
            Some(listener) => UnixServer::from_listener(
                listener,
                handler.clone(),
                cancel.clone(),
                uid,
                gid,
                clock,
            ),
            None => {
                UnixServer::new_user_private(
                    &config.socket,
                    handler.clone(),
                    cancel.clone(),
                    uid,
                    gid,
                    Arc::new(SystemClock::new()),
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
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        });
        server.run().await?;
        self.request_handler.cancel_current_turn().await;
        self.request_handler.shutdown_runtime().await?;
        Ok(())
    }
}

fn tempfile_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{label}-{}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::apply_exec_server_override;
    use crate::core::config::GrokHardeningConfig;

    #[test]
    fn exec_server_cli_override_is_additive_over_layered_config() {
        let mut off = GrokHardeningConfig::default();
        apply_exec_server_override(&mut off, false);
        assert!(!off.exec_server);
        apply_exec_server_override(&mut off, true);
        assert!(off.exec_server);

        let mut configured = GrokHardeningConfig {
            exec_server: true,
            ..Default::default()
        };
        apply_exec_server_override(&mut configured, false);
        assert!(configured.exec_server);
    }
}
