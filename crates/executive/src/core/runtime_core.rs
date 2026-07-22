//! Runtime core — host-agnostic agent bootstrap.
//!
//! `RuntimeCore` owns all agent-level state that exists independent of how
//! the process is deployed.  `DaemonHost`, `SystemdHost`, `ContainerHost`,
//! or a CLI one-shot host all share the same bootstrap path through
//! [`RuntimeCore::bootstrap`].
//!
//! Hosts own process-level concerns (PID files, socket binding, .env loading,
//! signal handling).  The core owns everything else.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use cognit::inference::pulse::{LlmPulse, PulseConfig};
use cognit::inference::scheduler::{
    LlmScheduler, RoutingRule, SchedulerConfig, SchedulerProviderConfig,
};
use fabric::evolution::LlmPurpose;
use fabric::CanonicalEventBus;
use fabric::Clock;

use kernel::chronos::SystemClock;

use crate::host::daemon::handler::RequestHandler;
use crate::host::daemon::DaemonConfig;
use cognit::composition::provider_registry::ProviderRegistry;

use dasein::perception::PerceptionEvent;

/// The agent runtime core — all agent-level state, host-independent.
pub struct RuntimeCore {
    pub app_config: crate::composition::config::AppConfig,
    pub registry: ProviderRegistry,
    pub daemon_config: DaemonConfig,
    pub event_bus: Arc<CanonicalEventBus>,
    pub pulse_handle: Option<(watch::Sender<bool>, JoinHandle<()>)>,
    pub request_handler: RequestHandler,
    pub cancel_token: CancellationToken,
}

impl RuntimeCore {
    /// Bootstrap the full agent runtime from configuration.
    ///
    /// This is **host-agnostic**.  It does NOT create PID files, load `.env`,
    /// create data directories, or bind sockets.  Those belong to the host
    /// layer ([`super::super::host::RuntimeHost`]).
    ///
    /// `config_path` — explicit config file path; falls back to layered config
    ///                  discovery when `None`.
    pub async fn bootstrap(config_path: Option<PathBuf>, enable_evolution: bool) -> Result<Self> {
        // ── AppConfig ───────────────────────────────────────────────
        // Layered base (defaults → /etc → user → project), then --config on top.
        let loaded = crate::composition::config::load_for_host(None, config_path.as_deref())?;
        // Resolve all enabled optional integrations before providers, storage,
        // sessions, or background workers start. Diagnostics contain only typed
        // config paths and credential reference identities, never secret values.
        let integrations = loaded
            .preflight_integrations(&crate::composition::config::EnvironmentCredentialResolver)
            .context("optional integration startup preflight")?;
        let app_config = loaded.value;
        tracing::info!(providers = %app_config.providers.len(), "Loaded config");

        // ── ProviderRegistry ────────────────────────────────────────
        let registry = ProviderRegistry::from_config(&app_config.cognit())?;
        let (default_provider_config, default_model) = registry.resolve("")?;

        // ── DaemonConfig ────────────────────────────────────────────
        let config = DaemonConfig {
            model: default_model.clone(),
            working_dir: app_config
                .bootstrap
                .working_dir
                .clone()
                .unwrap_or(
                    std::env::current_dir().context(
                        "bootstrap.working_dir is unset and the process cwd is unavailable",
                    )?,
                )
                .to_string_lossy()
                .to_string(),
            data_dir: app_config.bootstrap.data_dir.clone().map_or_else(
                || {
                    if app_config.deployment.mode == cognit::config::DeploymentMode::Production {
                        app_config
                            .deployment
                            .paths
                            .state
                            .to_string_lossy()
                            .to_string()
                    } else {
                        fabric::paths::xdg_data_dir().to_string_lossy().to_string()
                    }
                },
                |path| path.to_string_lossy().to_string(),
            ),
            system_prompt: app_config.agent.system_prompt.clone(),
            sandbox_preference: app_config
                .bootstrap
                .sandbox_preference
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
            conscious_arbitration_mode: crate::host::daemon::parse_conscious_arbitration_mode(
                app_config.bootstrap.conscious_arbitration_mode.as_deref(),
            )?,
            enable_evolution,
            mcp_servers: super::mcp_config::convert_mcp_servers(&app_config.mcp_servers),
            hooks: {
                // Honor --config: hooks must come from the same file(s) as the
                // main config, not always ~/.aletheon. (Fixes the hooks bug.)
                app_config.hooks.clone()
            },
            telegram: app_config.telegram.clone(),
            supplemental_memory: app_config.memory.supplemental.clone(),
            deployment: app_config.deployment.clone(),
            backpressure: app_config.backpressure.clone(),
            agent_admission: app_config.agent.admission.clone(),
            agent_max_iterations: app_config.agent.max_iterations,
            harness_kind: app_config.agent.harness_kind,
            integrations,
            embodiment_provider: Default::default(),
        };

        // ── Event bus ───────────────────────────────────────────────
        let bus = Arc::new(CanonicalEventBus::default());

        let cancel_token = CancellationToken::new();

        // ── LlmPulse ────────────────────────────────────────────────
        let pulse_handle = if !app_config.providers.is_empty() {
            let mut routing = std::collections::HashMap::new();
            routing.insert(LlmPurpose::Execute, default_provider_config.name.clone());
            routing.insert(LlmPurpose::Reflect, default_provider_config.name.clone());

            let scheduler_config = SchedulerConfig {
                providers: app_config
                    .providers
                    .iter()
                    .map(|p| SchedulerProviderConfig {
                        definition: p.clone(),
                        model: p.models.first().cloned().unwrap_or_default(),
                    })
                    .collect(),
                routing: routing
                    .into_iter()
                    .map(|(purpose, provider_name)| RoutingRule {
                        purpose,
                        provider_name,
                    })
                    .collect(),
                max_tokens: app_config.agent.max_tokens as u32,
                provider_timeouts: app_config.agent.provider_timeouts,
            };

            let scheduler_clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
            match LlmScheduler::new(&scheduler_config, scheduler_clock.clone()) {
                Ok(scheduler) => {
                    let scheduler = Arc::new(scheduler);
                    let pulse = LlmPulse::new(
                        scheduler,
                        bus.clone(),
                        PulseConfig::default(),
                        scheduler_clock,
                    );
                    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

                    let handle = tokio::spawn(async move {
                        pulse.run(shutdown_rx).await;
                    });

                    tracing::info!("LlmPulse started");
                    Some((shutdown_tx, handle))
                }
                Err(e) => {
                    tracing::warn!("Failed to create LlmScheduler, skipping LlmPulse: {}", e);
                    None
                }
            }
        } else {
            tracing::info!("No LLM providers configured, skipping LlmPulse");
            None
        };

        // ── Perception manager (gated) ──────────────────────────────
        // The old PerceptionBridge fed an "Engine" that was removed; its
        // injection receiver was dropped, which caused endless
        // "Engine receiver dropped" warnings and an unbounded buffer.
        // Until the perception→behavior loop is rewired (roadmap §T3), only
        // spawn the manager when explicitly enabled, and do not spawn the
        // bridge at all.
        if app_config.perception.enabled {
            let (event_tx, mut event_rx) = mpsc::channel::<PerceptionEvent>(256);
            let perception_config = &app_config.perception;
            let watch_paths: Vec<PathBuf> = perception_config
                .watch_paths
                .iter()
                .map(PathBuf::from)
                .collect();
            let enable_journald = perception_config.enable_journald;
            let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
            tokio::spawn(async move {
                let mut manager = dasein::perception::manager::PerceptionManager::new(
                    event_tx,
                    watch_paths,
                    enable_journald,
                    clock,
                );
                if let Err(e) = manager.start().await {
                    tracing::error!(error = %e, "Perception manager failed");
                }
            });
            // Drain-and-drop until §T3 wires a real consumer, so the manager's
            // sender does not back-pressure. (No behavior injection yet.)
            tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
        }

        // ── RequestHandler ──────────────────────────────────────────
        info!("Creating request handler...");
        let request_handler = RequestHandler::new(
            &config,
            Arc::new(crate::core::RegistryInferencePort::new(Arc::new(
                registry.clone(),
            ))),
            app_config.model_routing.clone(),
            app_config.model_aliases.clone(),
            app_config.goal_runtime.clone().unwrap_or_default(),
            app_config.pi_runtime.clone(),
            app_config.grok_hardening.clone(),
            app_config.sandbox_profiles.clone(),
            app_config.network_policy.clone(),
            app_config.agent_profiles.clone(),
            config.enable_evolution,
            Some(bus.clone()),
            cancel_token.clone(),
        )
        .await?;

        Ok(Self {
            app_config,
            registry,
            daemon_config: config,
            event_bus: bus,
            pulse_handle,
            request_handler,
            cancel_token,
        })
    }
}
