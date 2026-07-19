//! Windows PtyHost — ConPTY pseudo-console (H2).

use crate::error::HostError;
use crate::pty::{PtyChannel, PtyHost, PtySize};
use crate::receipt::HostReceipt;
use async_trait::async_trait;

pub struct WindowsPtyHost;
impl WindowsPtyHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PtyHost for WindowsPtyHost {
    async fn open(&self, _size: PtySize) -> Result<Box<dyn PtyChannel>, HostError> {
        Err(HostError::unsupported("conpty open"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract_unimplemented() {
        assert!(WindowsPtyHost::new()
            .open(PtySize { rows: 24, cols: 80 })
            .await
            .is_err());
    }
}
