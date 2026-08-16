//! Machine-scoped inference runtime owned by the `aletheon` composition root.
//!
//! This module deliberately owns no user session, workspace, approval, tool,
//! or sandbox surface. It resolves model specifications and serves inference
//! over the authenticated local core RPC transport.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::wiring::adapters::inference::ProviderRegistry;
use ::contracts::{LlmResponse, LlmStream};
use anyhow::Context;
use tokio_util::sync::CancellationToken;

use super::core_rpc::{CorePeerPolicy, CoreRpcServer};
use cognit::ports::inference::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};

/// The only adapter allowed to instantiate machine-scoped providers.
#[derive(Clone)]
pub struct RegistryInferencePort {
    registry: Arc<ProviderRegistry>,
}

impl RegistryInferencePort {
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl InferencePort for RegistryInferencePort {
    async fn acquire_provider_permit(
        &self,
        provider_key: &str,
    ) -> Result<Box<dyn ::contracts::memory::ProviderRequestPermit>, InferenceError> {
        use ::contracts::memory::ProviderBackpressurePort;
        crate::wiring::adapters::inference::backpressure::MachineProviderBackpressure::new(
            self.registry.backpressure_config_for_key(provider_key),
        )
        .acquire(provider_key)
        .await
        .map_err(InferenceError::from)
    }

    async fn observe_provider_retry_after(
        &self,
        provider_key: &str,
        retry_after_ms: Option<u64>,
    ) -> Result<(), InferenceError> {
        use ::contracts::memory::ProviderBackpressurePort;
        crate::wiring::adapters::inference::backpressure::MachineProviderBackpressure::new(
            self.registry.backpressure_config_for_key(provider_key),
        )
        .observe_retry_after(provider_key, retry_after_ms)
        .await;
        Ok(())
    }

    async fn provider_backpressure_metrics(
        &self,
    ) -> Result<
        std::collections::HashMap<String, cognit::inference::ProviderBackpressureSnapshot>,
        InferenceError,
    > {
        Ok(crate::wiring::adapters::inference::backpressure::all_provider_backpressure_snapshots())
    }

    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        let (config, model) = self
            .registry
            .resolve(model_spec)
            .map_err(InferenceError::from)?;
        let provider = self
            .registry
            .create_provider(&config, &model)
            .map_err(InferenceError::from)?;
        let runtime_facts = provider.runtime_facts();
        Ok(ModelCapabilities {
            provider_id: Some(config.name.clone()),
            transport: Some(format!("{:?}", config.transport).to_ascii_lowercase()),
            cache_reporting: runtime_facts.cache_reporting,
            model_spec: format!("{}/{}", config.name, model),
            display_name: provider.name().to_string(),
            max_context_tokens: provider.max_context_length(),
        })
    }

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
pub struct MachineInferenceRuntime {
    provider_registry: Arc<ProviderRegistry>,
    inference_server: Option<CoreRpcServer>,
    cancel: CancellationToken,
    socket_path: PathBuf,
}

impl MachineInferenceRuntime {
    pub async fn bootstrap(
        config_path: Option<&Path>,
        socket_path: PathBuf,
    ) -> anyhow::Result<Self> {
        // Passing no project directory is intentional: the system core may read
        // machine/user configuration layers plus an explicit operator file, but
        // never configuration from the caller's current workspace.
        let app_config = crate::config::load_for_host(None, config_path)?.value;
        let crate::config::AppConfig {
            telegram,
            memory: mnemosyne::supplemental_memory::MemoryConfig { supplemental, .. },
            mcp_servers,
            ..
        } = &app_config;
        if telegram.enabled || supplemental.enabled || !mcp_servers.is_empty() {
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
}
