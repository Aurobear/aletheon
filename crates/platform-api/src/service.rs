//! ServiceHost — OS service lifecycle management.

use crate::error::HostError;
use crate::receipt::HostReceipt;
use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    Failed,
    Unknown,
}

#[async_trait]
pub trait ServiceHost: Send + Sync {
    async fn status(&self, name: &str) -> Result<ServiceState, HostError>;
    async fn start(&self, name: &str) -> Result<HostReceipt, HostError>;
    async fn stop(&self, name: &str) -> Result<HostReceipt, HostError>;
    async fn restart(&self, name: &str) -> Result<HostReceipt, HostError>;
}
