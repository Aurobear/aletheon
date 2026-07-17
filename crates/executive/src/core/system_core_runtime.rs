//! Machine-scoped inference runtime.
//!
//! This module deliberately owns no user session, workspace, approval, tool,
//! or sandbox surface. It resolves model specifications and serves inference
//! over the authenticated local core RPC transport.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use cognit::r#impl::provider_registry::ProviderRegistry;
use fabric::{LlmResponse, LlmStream};
use tokio_util::sync::CancellationToken;

use crate::r#impl::core_rpc::{CorePeerPolicy, CoreRpcServer};
use crate::service::inference_port::{CoreInferenceRequest, InferenceError, InferencePort};

/// A model specification after authoritative machine-registry resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    pub provider: String,
    pub model: String,
}

/// The only adapter allowed to instantiate machine-scoped providers.
#[derive(Clone)]
pub struct RegistryInferencePort {
    registry: Arc<ProviderRegistry>,
}

impl RegistryInferencePort {
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self { registry }
    }

    pub fn resolve_model(&self, model_spec: &str) -> anyhow::Result<ResolvedModel> {
        let (provider, model) = self.registry.resolve(model_spec)?;
        Ok(ResolvedModel {
            provider: provider.name,
            model,
        })
    }

    /// Deterministic fixture used by cross-crate boundary tests.
    pub fn fixture_with_alias(alias: &str, target: &str) -> Self {
        let (provider_name, model) = target.split_once('/').unwrap_or(("openai", "gpt-test"));
        let mut config = cognit::config::CognitConfig::default();
        config.agent.default_provider = Some(provider_name.to_string());
        config.agent.default_model = Some(model.to_string());
        config.providers.push(cognit::config::ProviderConfig {
            name: provider_name.to_string(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: String::new(),
            transport: cognit::config::Transport::Openai,
            models: vec![model.to_string()],
            max_context_length: None,
            pricing: None,
        });
        config
            .model_aliases
            .insert(alias.to_string(), target.to_string());
        Self::new(Arc::new(
            ProviderRegistry::from_config(&config).expect("valid registry fixture"),
        ))
    }
}

#[async_trait::async_trait]
impl InferencePort for RegistryInferencePort {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        let provider = self
            .registry
            .resolve_and_create(&request.model_spec)
            .map_err(InferenceError::from)?;
        provider
            .complete(&request.messages, &request.tools)
            .await
            .map_err(Into::into)
    }

    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        let provider = self
            .registry
            .resolve_and_create(&request.model_spec)
            .map_err(InferenceError::from)?;
        provider
            .complete_stream(&request.messages, &request.tools)
            .await
            .map_err(Into::into)
    }
}

/// Machine-wide core process. The RPC server is the complete public runtime
/// surface; provider construction remains private behind `RegistryInferencePort`.
pub struct SystemCoreRuntime {
    provider_registry: Arc<ProviderRegistry>,
    inference_server: Option<CoreRpcServer>,
    cancel: CancellationToken,
    socket_path: PathBuf,
}

impl SystemCoreRuntime {
    pub async fn bootstrap(
        config_path: Option<&Path>,
        socket_path: PathBuf,
    ) -> anyhow::Result<Self> {
        // Passing no project directory is intentional: the system core may read
        // machine/user configuration layers plus an explicit operator file, but
        // never configuration from the caller's current workspace.
        let app_config = crate::core::config::load_for_host(None, config_path)?.value;
        let crate::core::config::AppConfig {
            telegram,
            memory: crate::core::config::MemoryConfig { gbrain, .. },
            mcp_servers,
            ..
        } = &app_config;
        if telegram.enabled || gbrain.enabled || !mcp_servers.is_empty() {
            anyhow::bail!("system core configuration contains user-scoped integration credentials");
        }
        let registry = Arc::new(ProviderRegistry::from_config(&app_config.cognit())?);
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating core socket directory {}", parent.display()))?;
        }
        let port: Arc<dyn InferencePort> =
            Arc::new(RegistryInferencePort::new(Arc::clone(&registry)));
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let server = CoreRpcServer::bind(
            &socket_path,
            port,
            CorePeerPolicy::new(uid, gid, std::iter::empty()),
        )
        .await?;
        Ok(Self {
            provider_registry: registry,
            inference_server: Some(server),
            cancel: CancellationToken::new(),
            socket_path,
        })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let server = self
            .inference_server
            .take()
            .context("system core inference server was already consumed")?;
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        });
        server.run(self.cancel.clone()).await
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn provider_count(&self) -> usize {
        self.provider_registry.provider_names().len()
    }

    pub fn fixture() -> Self {
        let port = RegistryInferencePort::fixture_with_alias("fast", "openai/gpt-test");
        Self {
            provider_registry: port.registry,
            inference_server: None,
            cancel: CancellationToken::new(),
            socket_path: PathBuf::from("/fixture/core.sock"),
        }
    }
}
