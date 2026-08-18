//! Machine-scoped inference port adapter.
//!
//! The only adapter allowed to instantiate machine-scoped providers. The host
//! composes it into the core runtime without importing provider construction
//! internals.

use std::sync::Arc;

use ::contracts::{LlmResponse, LlmStream};
use cognit::ports::inference::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};

use crate::backpressure::all_provider_backpressure_snapshots;
use crate::backpressure::MachineProviderBackpressure;
use crate::registry::ProviderRegistry;

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
        MachineProviderBackpressure::new(self.registry.backpressure_config_for_key(provider_key))
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
        MachineProviderBackpressure::new(self.registry.backpressure_config_for_key(provider_key))
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
        Ok(all_provider_backpressure_snapshots())
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
