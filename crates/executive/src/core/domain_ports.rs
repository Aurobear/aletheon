//! Executive-owned cognitive domain composition.

use fabric::AgoraOps;
use std::sync::Arc;

/// Cognitive domain ports are intentionally separate from KernelRuntime.
#[derive(Clone)]
pub struct DomainPorts {
    agora: Arc<dyn AgoraOps>,
}

impl DomainPorts {
    pub fn new(agora: Arc<dyn AgoraOps>) -> Self {
        Self { agora }
    }

    pub fn agora(&self) -> Arc<dyn AgoraOps> {
        self.agora.clone()
    }
}

impl std::fmt::Debug for DomainPorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DomainPorts")
            .field("agora", &"configured")
            .finish()
    }
}
