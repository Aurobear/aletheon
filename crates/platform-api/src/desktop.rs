//! DesktopHost — structured UI observation and input.

use crate::error::HostError;
use crate::receipt::HostReceipt;
use async_trait::async_trait;

#[async_trait]
pub trait DesktopHost: Send + Sync {
    async fn observe_supported(&self) -> Result<bool, HostError>;
    async fn input_supported(&self) -> Result<bool, HostError>;
    async fn screenshot(&self) -> Result<Vec<u8>, HostError>;
    async fn click(&self, x: u32, y: u32) -> Result<HostReceipt, HostError>;
    async fn type_text(&self, text: &str) -> Result<HostReceipt, HostError>;
}
