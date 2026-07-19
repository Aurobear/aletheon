//! macOS ServiceHost — launchd plist management (H3).

use crate::error::HostError;
use crate::receipt::HostReceipt;
use crate::service::{ServiceHost, ServiceState};
use async_trait::async_trait;

pub struct MacOSServiceHost;
impl MacOSServiceHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ServiceHost for MacOSServiceHost {
    async fn status(&self, _n: &str) -> Result<ServiceState, HostError> {
        Err(HostError::unsupported("launchd"))
    }
    async fn start(&self, _n: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("launchd"))
    }
    async fn stop(&self, _n: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("launchd"))
    }
    async fn restart(&self, _n: &str) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("launchd"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract() {
        assert!(MacOSServiceHost::new()
            .status("com.apple.sshd")
            .await
            .is_err());
    }
}
