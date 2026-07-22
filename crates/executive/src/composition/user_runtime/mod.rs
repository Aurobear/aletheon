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
    pi_runtime: cognit::config::PiRuntimeConfig,
    grok_hardening: crate::composition::config::GrokHardeningConfig,
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
        let integrations = loaded
            .preflight_integrations(&crate::composition::config::EnvironmentCredentialResolver)
            .context("optional integration startup preflight")?;
        let mut app = loaded.value;
        // CLI activation is additive: an absent flag preserves the layered
        // config value, while `--execd` can only enable the backend.
        apply_execd_override(&mut app.grok_hardening, enable_execd);
        let crate::composition::config::AppConfig {
            memory: crate::composition::config::MemoryConfig { gbrain, .. },
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
            mcp_servers: crate::core::mcp_config::convert_mcp_servers(&app.mcp_servers),
            hooks: app.hooks.clone(),
            telegram: app.telegram.clone(),
            gbrain_memory: gbrain.clone(),
            deployment,
            backpressure: app.backpressure.clone(),
            agent_admission: app.agent.admission.clone(),
            agent_max_iterations: app.agent.max_iterations,
            harness_kind: app.agent.harness_kind,
            integrations,
            embodiment_provider: Default::default(),
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
        let mut config = Self::load(None, paths, socket, false, false)
            .expect("default user runtime fixture config must load");
        config.request.mcp_servers.clear();
        config.request.telegram.enabled = false;
        config.request.gbrain_memory.enabled = false;

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

fn apply_execd_override(config: &mut crate::composition::config::GrokHardeningConfig, cli_enabled: bool) {
    config.execd |= cli_enabled;
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
            config.network_policy.clone(),
            config.agent_profiles.clone(),
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
    use super::apply_execd_override;
    use crate::composition::config::GrokHardeningConfig;

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
}
