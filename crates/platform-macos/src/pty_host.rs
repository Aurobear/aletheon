//! macOS PtyHost — Unix98 PTY (H3).

use platform_api::error::HostError;
use platform_api::pty::{PtyChannel, PtyHost, PtySize};
use async_trait::async_trait;

pub struct MacOSPtyHost;
impl MacOSPtyHost { pub fn new() -> Self { Self } }

#[async_trait]
impl PtyHost for MacOSPtyHost {
    async fn open(&self, _size: PtySize) -> Result<Box<dyn PtyChannel>, HostError> { Err(HostError::unsupported("pty")) }
}
#[cfg(test)] mod tests { use super::*; #[tokio::test] async fn contract() { assert!(MacOSPtyHost::new().open(PtySize{rows:24,cols:80}).await.is_err()); } }
