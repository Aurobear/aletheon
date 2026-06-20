pub mod cache_shape;
pub mod debug_handler;
pub mod handler;
pub mod mcp_embedded;
pub mod model_router;
pub mod prefix_builder;
pub mod server;
pub mod session_manager;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use aletheon_abi::evolution::LlmPurpose;
use aletheon_brain::r#impl::llm::pulse::{LlmPulse, PulseConfig};
use aletheon_brain::r#impl::llm::scheduler::{
    LlmScheduler, RoutingRule, SchedulerConfig, SchedulerProviderConfig,
};
use aletheon_comm::KernelEventBus;

use crate::ProviderRegistry;

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
    /// MCP server definitions loaded from config (passed through to McpManager at handler init).
    pub mcp_servers: Vec<aletheon_body::r#impl::mcp::config::McpServerConfig>,
    /// Hook script configuration from the [hooks] config section.
    pub hooks: crate::core::config::HooksConfig,
}

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

/// Default config file search paths.
#[allow(dead_code)]
fn default_config_path() -> PathBuf {
    // 1. ~/.aletheon/config.toml
    let path = aletheon_abi::paths::config_file();
    if path.exists() {
        return path;
    }
    // 2. /etc/agentd/config.toml
    PathBuf::from("/etc/agentd/config.toml")
}

/// Run the daemon with the given CLI arguments.
pub async fn run(
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
) -> Result<()> {
    // Write PID file for daemon management
    let pid_file = PathBuf::from("/tmp/aletheon/aletheond.pid");
    if let Some(parent) = pid_file.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&pid_file, std::process::id().to_string()).ok();

    // Load .env file
    let env_path = env_path.unwrap_or_else(|| {
        // Search: ~/.aletheon/.env
        let path = aletheon_abi::paths::env_file();
        if path.exists() {
            return path;
        }
        PathBuf::from(".env")
    });
    load_dotenv(&env_path);

    // Load AppConfig with layered merging (defaults -> global -> project)
    let app_config = if let Some(ref path) = config_path {
        aletheon_brain::config::AppConfig::load_or_default(path)
    } else {
        aletheon_brain::config::AppConfig::load_layered(None)
    };

    tracing::info!(providers = %app_config.providers.len(), "Loaded config");

    // Build provider registry
    let registry = ProviderRegistry::from_config(&app_config)?;

    // Resolve default provider for legacy DaemonConfig fields
    let (default_provider_config, default_model) = registry.resolve("")?;

    let config = DaemonConfig {
        api_key: default_provider_config.api_key.clone(),
        api_url: default_provider_config.base_url.clone(),
        model: default_model.clone(),
        working_dir: std::env::var("AGENT_WORKING_DIR").unwrap_or_else(|_| "/tmp".to_string()),
        data_dir: std::env::var("AGENT_DATA_DIR").unwrap_or_else(|_| {
            aletheon_abi::paths::xdg_data_dir()
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
            .map(|s| aletheon_body::r#impl::mcp::config::McpServerConfig {
                name: s.name.clone(),
                transport: match s.transport.as_str() {
                    "stdio" => aletheon_body::r#impl::mcp::config::McpTransportConfig::Stdio {
                        command: s.command.clone().unwrap_or_default(),
                        args: Vec::new(),
                    },
                    "http" => {
                        aletheon_body::r#impl::mcp::config::McpTransportConfig::StreamableHttp {
                            url: s.url.clone().unwrap_or_default(),
                        }
                    }
                    "sse" => aletheon_body::r#impl::mcp::config::McpTransportConfig::Sse {
                        url: s.url.clone().unwrap_or_default(),
                    },
                    _ => aletheon_body::r#impl::mcp::config::McpTransportConfig::Stdio {
                        command: s.command.clone().unwrap_or_default(),
                        args: Vec::new(),
                    },
                },
                trust: aletheon_body::r#impl::mcp::config::McpTrustLevel::LocalTrusted,
                enabled: true,
            })
            .collect(),
        hooks: {
            // Load hooks config from the runtime's own AppConfig
            let rt_config = crate::core::config::AppConfig::load_layered(None);
            rt_config.hooks
        },
    };

    // Ensure data directory exists
    tracing::info!(data_dir = %config.data_dir, "Creating data directory...");
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create data dir '{}': {}", config.data_dir, e))?;

    tracing::info!(
        provider = %default_provider_config.name,
        model = %default_model,
        data_dir = %config.data_dir,
        "Starting agentd"
    );

    // Create event bus and start LlmPulse if LLM providers are configured
    let bus: Arc<dyn aletheon_abi::EventBus> = Arc::new(KernelEventBus::new(4096));

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
                        aletheon_brain::config::Transport::Anthropic => "anthropic".to_string(),
                        aletheon_brain::config::Transport::Openai => "openai".to_string(),
                        aletheon_brain::config::Transport::Auto => "openai".to_string(),
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

    // Start perception manager and bridge
    let (event_tx, event_rx) =
        mpsc::channel::<aletheon_self::r#impl::perception::PerceptionEvent>(256);
    let (injection_tx, injection_rx) =
        mpsc::channel::<aletheon_self::r#impl::perception::bridge::PerceptionInjection>(64);

    let perception_config = &app_config.perception;
    let watch_paths: Vec<PathBuf> = perception_config
        .watch_paths
        .iter()
        .map(PathBuf::from)
        .collect();
    let enable_journald = perception_config.enable_journald;
    tokio::spawn(async move {
        let mut manager = aletheon_self::r#impl::perception::manager::PerceptionManager::new(
            event_tx,
            watch_paths,
            enable_journald,
        );
        if let Err(e) = manager.start().await {
            tracing::error!(error = %e, "Perception manager failed");
        }
    });

    // Start perception bridge
    let mut bridge =
        aletheon_self::r#impl::perception::bridge::PerceptionBridge::new(event_rx, injection_tx);
    tokio::spawn(async move {
        bridge.run().await;
    });

    info!("Creating request handler...");
    let request_handler = handler::RequestHandler::new(&config, &registry, app_config.model_routing.clone(), injection_rx, Some(bus.clone())).await?;
    info!(socket = %socket.display(), "Binding unix socket...");

    let mut unix_server = server::UnixServer::new(&socket, request_handler).await?;

    // Clean up PID file and pulse on exit
    let pid_file_clone = pid_file.clone();
    let pulse_handle_clone = pulse_handle.as_ref().map(|(tx, _)| tx.clone());
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        // Signal LlmPulse to shut down
        if let Some(tx) = pulse_handle_clone {
            let _ = tx.send(true);
        }
        std::fs::remove_file(&pid_file_clone).ok();
        std::process::exit(0);
    });

    unix_server.run().await?;

    // Graceful shutdown: stop LlmPulse
    if let Some((shutdown_tx, handle)) = pulse_handle {
        let _ = shutdown_tx.send(true);
        // Give pulse a moment to finish
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    // Clean up PID file on normal exit
    std::fs::remove_file(&pid_file).ok();

    Ok(())
}
