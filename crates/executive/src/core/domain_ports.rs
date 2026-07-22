//! Executive-owned cognitive domain composition.

use fabric::AgoraService;
use std::sync::Arc;

/// Cognitive domain ports are intentionally separate from KernelRuntime.
#[derive(Clone)]
pub struct DomainPorts {
    agora: Arc<dyn AgoraService>,
    metacog: Arc<dyn metacog::MetacogService>,
    corpus: Arc<dyn corpus::CorpusService>,
    cognition: Arc<dyn crate::application::harness_factory::CognitiveSessionFactory>,
}

impl DomainPorts {
    pub fn new(
        agora: Arc<dyn AgoraService>,
        metacog: Arc<dyn metacog::MetacogService>,
        corpus: Arc<dyn corpus::CorpusService>,
        cognition: Arc<dyn crate::application::harness_factory::CognitiveSessionFactory>,
    ) -> Self {
        Self {
            agora,
            metacog,
            corpus,
            cognition,
        }
    }

    pub fn agora(&self) -> Arc<dyn AgoraService> {
        self.agora.clone()
    }

    pub fn metacog(&self) -> Arc<dyn metacog::MetacogService> {
        self.metacog.clone()
    }

    pub fn corpus(&self) -> Arc<dyn corpus::CorpusService> {
        self.corpus.clone()
    }

    pub fn cognition(
        &self,
    ) -> Arc<dyn crate::application::harness_factory::CognitiveSessionFactory> {
        self.cognition.clone()
    }
}

impl std::fmt::Debug for DomainPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DomainPorts")
            .field("agora", &"configured")
            .field("metacog", &"configured")
            .field("corpus", &"configured")
            .field("cognition", &"configured")
            .finish()
    }
}
