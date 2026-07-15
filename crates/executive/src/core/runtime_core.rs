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

use anyhow::Result;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use cognit::r#impl::llm::pulse::{LlmPulse, PulseConfig};
use cognit::r#impl::llm::scheduler::{
    LlmScheduler, RoutingRule, SchedulerConfig, SchedulerProviderConfig,
};
use fabric::evolution::LlmPurpose;
use fabric::Clock;
use fabric::CommunicationBus;

use aletheon_kernel::chronos::SystemClock;

use crate::r#impl::daemon::handler::RequestHandler;
use crate::r#impl::daemon::DaemonConfig;
use cognit::r#impl::provider_registry::ProviderRegistry;

use dasein::r#impl::perception::PerceptionEvent;

/// The agent runtime core — all agent-level state, host-independent.
pub struct RuntimeCore {
    pub app_config: cognit::config::AppConfig,
    pub registry: ProviderRegistry,
    pub daemon_config: DaemonConfig,
    pub event_bus: Arc<CommunicationBus>,
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
        let mut app_config = cognit::config::AppConfig::load_layered(None);
        if let Some(ref path) = config_path {
            app_config.merge(cognit::config::AppConfig::load_or_default(path));
        }
        tracing::info!(providers = %app_config.providers.len(), "Loaded config");

        // ── ProviderRegistry ────────────────────────────────────────
        let registry = ProviderRegistry::from_config(&app_config)?;
        let (default_provider_config, default_model) = registry.resolve("")?;

        // ── DaemonConfig ────────────────────────────────────────────
        let config = DaemonConfig {
            api_key: default_provider_config.api_key.clone(),
            api_url: default_provider_config.base_url.clone(),
            model: default_model.clone(),
            working_dir: std::env::var("AGENT_WORKING_DIR").unwrap_or_else(|_| "/tmp".to_string()),
            data_dir: std::env::var("AGENT_DATA_DIR").unwrap_or_else(|_| {
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
            }),
            system_prompt: std::env::var("AGENT_SYSTEM_PROMPT")
                .unwrap_or_else(|_| app_config.agent.system_prompt.clone()),
            sandbox_preference: std::env::var("AGENT_SANDBOX_PREFERENCE")
                .unwrap_or_else(|_| "auto".to_string()),
            enable_evolution,
            mcp_servers: app_config
                .mcp_servers
                .iter()
                .map(|s| corpus::tools::mcp::config::McpServerConfig {
                    name: s.name.clone(),
                    transport: match s.transport.as_str() {
                        "stdio" => corpus::tools::mcp::config::McpTransportConfig::Stdio {
                            command: s.command.clone().unwrap_or_default(),
                            args: Vec::new(),
                        },
                        "http" => corpus::tools::mcp::config::McpTransportConfig::StreamableHttp {
                            url: s.url.clone().unwrap_or_default(),
                        },
                        "sse" => corpus::tools::mcp::config::McpTransportConfig::Sse {
                            url: s.url.clone().unwrap_or_default(),
                        },
                        _ => corpus::tools::mcp::config::McpTransportConfig::Stdio {
                            command: s.command.clone().unwrap_or_default(),
                            args: Vec::new(),
                        },
                    },
                    trust: corpus::tools::mcp::config::McpTrustLevel::LocalTrusted,
                    enabled: true,
                    bearer_token_env: s.bearer_token_env.clone(),
                })
                .collect(),
            hooks: {
                // Honor --config: hooks must come from the same file(s) as the
                // main config, not always ~/.aletheon. (Fixes the hooks bug.)
                let rt_config = if let Some(ref path) = config_path {
                    crate::core::config::AppConfig::load_or_default(path)
                } else {
                    crate::core::config::AppConfig::load_layered(None)
                };
                rt_config.hooks
            },
            telegram: app_config.telegram.clone(),
            gbrain_memory: app_config.memory.gbrain.clone(),
            deployment: app_config.deployment.clone(),
        };

        // ── Event bus ───────────────────────────────────────────────
        let bus: Arc<CommunicationBus> = Arc::new(CommunicationBus::new());

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
                        name: p.name.clone(),
                        base_url: p.base_url.clone(),
                        api_key: p.api_key.clone(),
                        kind: match p.transport {
                            cognit::config::Transport::Anthropic => "anthropic".to_string(),
                            cognit::config::Transport::Openai => "openai".to_string(),
                            cognit::config::Transport::Auto => "openai".to_string(),
                        },
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
            };

            match LlmScheduler::new(&scheduler_config) {
                Ok(scheduler) => {
                    let scheduler = Arc::new(scheduler);
                    let pulse = LlmPulse::new(
                        scheduler,
                        bus.clone(),
                        PulseConfig::default(),
                        Arc::new(aletheon_kernel::chronos::SystemClock::new()),
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
                let mut manager = dasein::r#impl::perception::manager::PerceptionManager::new(
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
            &registry,
            app_config.model_routing.clone(),
            app_config.goal_runtime.clone().unwrap_or_default(),
            app_config.pi_runtime.clone(),
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
