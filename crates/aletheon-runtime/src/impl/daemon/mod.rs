pub mod handler;
pub mod server;

use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::ProviderRegistry;

/// Daemon configuration, migrated from argosd.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub sandbox_preference: String,
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
fn default_config_path() -> PathBuf {
    // 1. ~/.argos/config.toml
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".argos/config.toml");
        if path.exists() {
            return path;
        }
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
    // Load .env file
    let env_path = env_path.unwrap_or_else(|| {
        // Search: ~/.argos/.env
        if let Some(home) = std::env::var_os("HOME") {
            let path = PathBuf::from(home).join(".argos/.env");
            if path.exists() {
                return path;
            }
        }
        PathBuf::from(".env")
    });
    load_dotenv(&env_path);

    // Load AppConfig from TOML
    let config_path = config_path.unwrap_or_else(default_config_path);
    let app_config = aletheon_brain_core::config::AppConfig::load_or_default(&config_path);

    tracing::info!(path = %config_path.display(), providers = %app_config.providers.len(), "Loaded config");

    // Build provider registry
    let registry = ProviderRegistry::from_config(&app_config)?;

    // Resolve default provider for legacy DaemonConfig fields
    let (default_provider_config, default_model) = registry.resolve("")?;

    let config = DaemonConfig {
        api_key: default_provider_config.api_key.clone(),
        api_url: default_provider_config.base_url.clone(),
        model: default_model.clone(),
        working_dir: std::env::var("AGENT_WORKING_DIR")
            .unwrap_or_else(|_| "/tmp".to_string()),
        data_dir: std::env::var("AGENT_DATA_DIR")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                format!("{}/.local/share/argos", home)
            }),
        system_prompt: std::env::var("AGENT_SYSTEM_PROMPT")
            .unwrap_or_else(|_| "You are a helpful system assistant.".to_string()),
        sandbox_preference: std::env::var("AGENT_SANDBOX_PREFERENCE")
            .unwrap_or_else(|_| "auto".to_string()),
    };

    // Ensure data directory exists
    tracing::info!(data_dir = %config.data_dir, "Creating data directory...");
    std::fs::create_dir_all(&config.data_dir).map_err(|e| {
        anyhow::anyhow!("Failed to create data dir '{}': {}", config.data_dir, e)
    })?;

    tracing::info!(
        provider = %default_provider_config.name,
        model = %default_model,
        data_dir = %config.data_dir,
        "Starting agentd"
    );

    // Start perception manager and bridge
    let (event_tx, event_rx) = mpsc::channel::<aletheon_self_field::r#impl::perception::PerceptionEvent>(256);
    let (injection_tx, injection_rx) = mpsc::channel::<aletheon_self_field::r#impl::perception::bridge::PerceptionInjection>(64);

    let watch_paths = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/var/log"),
    ];
    tokio::spawn(async move {
        let mut manager = aletheon_self_field::r#impl::perception::manager::PerceptionManager::new(
            event_tx,
            watch_paths,
            true, // enable journald
        );
        if let Err(e) = manager.start().await {
            tracing::error!(error = %e, "Perception manager failed");
        }
    });

    // Start perception bridge
    let mut bridge = aletheon_self_field::r#impl::perception::bridge::PerceptionBridge::new(event_rx, injection_tx);
    tokio::spawn(async move {
        bridge.run().await;
    });

    info!("Creating request handler...");
    let request_handler = handler::RequestHandler::new(&config, &registry, injection_rx).await?;
    info!(socket = %socket.display(), "Binding unix socket...");

    let unix_server = server::UnixServer::new(&socket, request_handler).await?;
    unix_server.run().await?;

    Ok(())
}
