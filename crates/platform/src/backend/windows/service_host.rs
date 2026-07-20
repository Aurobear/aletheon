//! Windows ServiceHost — Service Control Manager (H2).

use crate::error::HostError;
use crate::receipt::HostReceipt;
use crate::service::{ServiceHost, ServiceState};
use async_trait::async_trait;

pub struct WindowsServiceHost;
impl WindowsServiceHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ServiceHost for WindowsServiceHost {
    async fn status(&self, _name: &str) -> Result<ServiceState, HostError> {
        Err(HostError::unsupported("scm status"))
    }
    async fn start(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("scm start"))
    }
    async fn stop(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("scm stop"))
    }
    async fn restart(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("scm restart"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract_unimplemented() {
        assert!(WindowsServiceHost::new().status("WinRM").await.is_err());
    }
}
