//! PtyHost — interactive pseudo-terminal.

use crate::error::HostError;
use crate::receipt::HostReceipt;
use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

#[async_trait]
pub trait PtyHost: Send + Sync {
    async fn open(&self, size: PtySize) -> Result<Box<dyn PtyChannel>, HostError>;
}

#[async_trait]
pub trait PtyChannel: Send {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, HostError>;
    async fn write(&mut self, data: &[u8]) -> Result<usize, HostError>;
    async fn resize(&mut self, size: PtySize) -> Result<HostReceipt, HostError>;
    async fn close(self: Box<Self>) -> Result<HostReceipt, HostError>;
}
