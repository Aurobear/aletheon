//! Linux ServiceHost — systemd D-Bus via zbus (H1-05).

use platform_api::error::HostError;
use platform_api::receipt::HostReceipt;
use platform_api::service::{ServiceHost, ServiceState};
use async_trait::async_trait;

pub struct LinuxServiceHost;

impl LinuxServiceHost {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ServiceHost for LinuxServiceHost {
    async fn status(&self, _name: &str) -> Result<ServiceState, HostError> {
        Err(HostError::unsupported("service status"))
    }
    async fn start(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("service start"))
    }
    async fn stop(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("service stop"))
    }
    async fn restart(&self, _name: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("service restart"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn service_host_contract_unimplemented() {
        let host = LinuxServiceHost::new();
        let result = host.status("aletheon.service").await;
        assert!(result.is_err());
    }
}
