//! Linux PTY host (H1-04).

use platform_api::error::HostError;
use platform_api::pty::{PtyChannel, PtyHost, PtySize};
use platform_api::receipt::HostReceipt;
use async_trait::async_trait;

pub struct LinuxPtyHost;

impl LinuxPtyHost {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl PtyHost for LinuxPtyHost {
    async fn open(&self, _size: PtySize) -> Result<Box<dyn PtyChannel>, HostError> {
        Err(HostError::unsupported("pty open"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pty_host_contract_unimplemented() {
        let host = LinuxPtyHost::new();
        let result = host.open(PtySize { rows: 24, cols: 80 }).await;
        assert!(result.is_err());
    }
}
