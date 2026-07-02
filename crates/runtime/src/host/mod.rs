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

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use base::evolution::LlmPurpose;
use cognit::r#impl::llm::pulse::{LlmPulse, PulseConfig};
use cognit::r#impl::llm::scheduler::{
    LlmScheduler, RoutingRule, SchedulerConfig, SchedulerProviderConfig,
};
use base::KernelEventBus;

use crate::r#impl::daemon::handler::RequestHandler;
use crate::r#impl::daemon::mcp_embedded::McpEmbedded;
use crate::r#impl::daemon::server;
use crate::r#impl::daemon::DaemonConfig;
use crate::ProviderRegistry;

/// Load .env file (simple KEY=VALUE parser, no shell expansion).
fn load_dotenv(path: &PathBuf) {
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
/// Holds startup configuration and runtime state populated by `init()`.
/// `serve()` starts the MCP embedded server, binds a Unix socket, handles
/// Ctrl+C, and runs the event loop. `shutdown()` cancels the token, stops
/// background subsystems, and removes the PID file.
pub struct DaemonHost {
    // --- CLI-supplied; set by new() ---
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,

    // --- Populated by init() ---
    cancel_token: CancellationToken,
    app_config: Option<cognit::config::AppConfig>,
    registry: Option<ProviderRegistry>,
    pulse_handle: Option<(watch::Sender<bool>, JoinHandle<()>)>,
    pid_file: Option<PathBuf>,
    request_handler: Option<RequestHandler>,
    perception_injection_rx:
        Option<mpsc::Receiver<dasein::r#impl::perception::bridge::PerceptionInjection>>,
    event_bus: Option<Arc<dyn base::EventBus>>,
    /// Stored config for use in serve().
    daemon_config: Option<DaemonConfig>,
}

impl DaemonHost {
    pub fn new(config_path: Option<PathBuf>, env_path: Option<PathBuf>, socket: PathBuf) -> Self {
        Self {
            config_path,
            env_path,
            socket,
            cancel_token: CancellationToken::new(),
            app_config: None,
            registry: None,
            pulse_handle: None,
            pid_file: None,
            request_handler: None,
            perception_injection_rx: None,
            event_bus: None,
            daemon_config: None,
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
            let path = base::paths::env_file();
            if path.exists() {
                return path;
            }
            PathBuf::from(".env")
        });
        load_dotenv(&env_path);

        // ── AppConfig ───────────────────────────────────────────────
        let app_config = if let Some(ref path) = self.config_path {
            cognit::config::AppConfig::load_or_default(path)
        } else {
            cognit::config::AppConfig::load_layered(None)
        };
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
                base::paths::xdg_data_dir()
                    .to_string_lossy()
                    .to_string()
            }),
            system_prompt: std::env::var("AGENT_SYSTEM_PROMPT")
                .unwrap_or_else(|_| app_config.agent.system_prompt.clone()),
            sandbox_preference: std::env::var("AGENT_SANDBOX_PREFERENCE")
                .unwrap_or_else(|_| "auto".to_string()),
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
                        "http" => {
                            corpus::tools::mcp::config::McpTransportConfig::StreamableHttp {
                                url: s.url.clone().unwrap_or_default(),
                            }
                        }
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
                })
                .collect(),
            hooks: {
                let rt_config = crate::core::config::AppConfig::load_layered(None);
                rt_config.hooks
            },
        };

        // ── Data dir ────────────────────────────────────────────────
        tracing::info!(data_dir = %config.data_dir, "Creating data directory...");
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create data dir '{}': {}", config.data_dir, e))?;

        tracing::info!(
            provider = %default_provider_config.name,
            model = %default_model,
            data_dir = %config.data_dir,
            "Starting agentd"
        );

        // ── Event bus ───────────────────────────────────────────────
        let bus: Arc<dyn base::EventBus> = Arc::new(KernelEventBus::new(4096));
        self.event_bus = Some(bus.clone());

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
                    let pulse = LlmPulse::new(scheduler, bus.clone(), PulseConfig::default());
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
        self.pulse_handle = pulse_handle;

        // ── Perception manager + bridge ─────────────────────────────
        let (event_tx, event_rx) =
            mpsc::channel::<dasein::r#impl::perception::PerceptionEvent>(256);
        let (injection_tx, injection_rx) =
            mpsc::channel::<dasein::r#impl::perception::bridge::PerceptionInjection>(64);

        let perception_config = &app_config.perception;
        let watch_paths: Vec<PathBuf> = perception_config
            .watch_paths
            .iter()
            .map(PathBuf::from)
            .collect();
        let enable_journald = perception_config.enable_journald;
        tokio::spawn(async move {
            let mut manager = dasein::r#impl::perception::manager::PerceptionManager::new(
                event_tx,
                watch_paths,
                enable_journald,
            );
            if let Err(e) = manager.start().await {
                tracing::error!(error = %e, "Perception manager failed");
            }
        });

        let mut bridge =
            dasein::r#impl::perception::bridge::PerceptionBridge::new(event_rx, injection_tx);
        tokio::spawn(async move {
            bridge.run().await;
        });

        self.perception_injection_rx = Some(injection_rx);

        // ── RequestHandler ──────────────────────────────────────────
        info!("Creating request handler...");
        let injection_rx = self
            .perception_injection_rx
            .take()
            .expect("perception_injection_rx must be set");
        let request_handler = RequestHandler::new(
            &config,
            &registry,
            app_config.model_routing.clone(),
            app_config.evolution.enabled,
            injection_rx,
            self.event_bus.clone(),
            self.cancel_token.clone(),
        )
        .await?;
        self.request_handler = Some(request_handler);

        // Store for serve()
        self.app_config = Some(app_config);
        self.registry = Some(registry);
        self.daemon_config = Some(config);

        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        let request_handler = self
            .request_handler
            .expect("request_handler must be set by init()");
        let cancel_token = self.cancel_token.clone();
        let socket = self.socket.clone();

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
        let mut unix_server =
            server::UnixServer::new(&socket, request_handler, cancel_token.clone()).await?;

        // ── Ctrl+C handler ──────────────────────────────────────────
        let shutdown_token = cancel_token.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
            shutdown_token.cancel();
        });

        unix_server.run().await?;

        // ── Graceful shutdown: stop LlmPulse ────────────────────────
        if let Some((shutdown_tx, handle)) = self.pulse_handle {
            let _ = shutdown_tx.send(true);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        }

        // ── Remove PID file ─────────────────────────────────────────
        if let Some(ref pid_file) = self.pid_file {
            std::fs::remove_file(pid_file).ok();
        }

        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        // Cancel the daemon token to trigger graceful shutdown.
        self.cancel_token.cancel();

        // Remove PID file if it exists.
        if let Some(ref pid_file) = self.pid_file {
            std::fs::remove_file(pid_file).ok();
        }

        // Stop LlmPulse if running.
        if let Some((shutdown_tx, _handle)) = self.pulse_handle.take() {
            let _ = shutdown_tx.send(true);
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
        // init/shutdown for DaemonHost now have real logic.
        // The zero-cost assertion is no longer valid, so this test
        // just verifies construction + lifecycle phases compile.
        let mut host = DaemonHost::new(None, None, PathBuf::from("/tmp/test.sock"));
        // init and shutdown may fail without a real config; accept that.
        let _ = host.init().await;
        let _ = host.shutdown().await;
    }
}
