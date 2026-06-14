pub mod proc_source;
pub mod inotify_source;
pub mod journald_source;
pub mod ebpf_source;
pub mod bottleneck_detector;

use async_trait::async_trait;
use super::PerceptionEvent;

/// Trait for perception sources.
#[async_trait]
pub trait PerceptionSource: Send + Sync {
    /// Source name for logging.
    fn name(&self) -> &str;

    /// Poll for new events. Returns empty vec if nothing new.
    async fn poll(&mut self) -> anyhow::Result<Vec<PerceptionEvent>>;

    /// Check if this source is available on the current system.
    fn is_available(&self) -> bool;
}
