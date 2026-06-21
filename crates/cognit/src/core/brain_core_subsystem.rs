//! Subsystem implementation for BrainCore.

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use base::{Subsystem, SubsystemContext, SubsystemHealth, Version};

use super::BrainCore;

#[async_trait]
impl Subsystem for BrainCore {
    fn name(&self) -> &str {
        "brain_core"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        info!("BrainCore initializing");
        self.initialized = true;
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if !self.initialized {
            return SubsystemHealth::Degraded {
                reason: "Not yet initialized".to_string(),
            };
        }
        SubsystemHealth::Healthy
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("BrainCore shutting down");
        self.world_model.clear();
        self.initialized = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}
