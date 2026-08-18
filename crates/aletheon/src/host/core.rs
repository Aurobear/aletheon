//! Machine-scoped inference runtime owned by the `aletheon` composition root.
//!
//! This module deliberately owns no user session, workspace, approval, tool,
//! or sandbox surface. It resolves model specifications and serves inference
//! over the authenticated local core RPC transport. Provider construction and
//! the machine-scoped inference port live in the `adapters-inference` crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adapters_inference::registry::ProviderRegistry;
use adapters_inference::{CorePeerPolicy, CoreRpcServer, RegistryInferencePort};
use anyhow::Context;
use cognit::ports::inference::InferencePort;
use tokio_util::sync::CancellationToken;

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
