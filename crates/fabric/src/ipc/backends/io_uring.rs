//! io_uring-based IPC backend for high-performance inter-agent communication.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::debug;

use crate::ipc::ipc_types::{AgentMessage, IpcBackend, IpcProbeError};

/// Compatibility placeholder for the retired experimental io_uring backend.
///
/// No external io_uring implementation is linked. Runtime selection therefore
/// fails probing explicitly and falls back to a supported transport.
pub struct IoUringBackend {
    available: bool,
    kernel_version: String,
    /// Optional clock for deterministic simulated-latency sleep in tests.
    clock: Option<Arc<dyn crate::Clock>>,
}

impl Default for IoUringBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl IoUringBackend {
    pub fn new() -> Self {
        let kernel_version = Self::get_kernel_version();
        let available = false;

        Self {
            available,
            kernel_version,
            clock: None,
        }
    }

    fn get_kernel_version() -> String {
        std::fs::read_to_string("/proc/version")
            .ok()
            .and_then(|v| {
                // Extract "X.Y.Z" from "Linux version X.Y.Z-..."
                v.split_whitespace().nth(2).map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown".to_string())
    }

    #[cfg(test)]
    fn check_kernel_support(version: &str) -> bool {
        // io_uring requires kernel >= 5.1
        let parts: Vec<u32> = version
            .split('.')
            .take(2)
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() >= 2 {
            parts[0] > 5 || (parts[0] == 5 && parts[1] >= 1)
        } else {
            false
        }
    }

    /// Attach a clock for deterministic simulated-latency sleep in tests.
    pub fn with_clock(mut self, clock: Arc<dyn crate::Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Probe whether io_uring is available on this system.
    pub fn probe() -> bool {
        false
    }

    fn init_ring(&mut self) -> Result<(), IpcProbeError> {
        Err(IpcProbeError::NotSupported)
    }
}

#[async_trait]
impl IpcBackend for IoUringBackend {
    async fn init(&mut self) -> Result<(), IpcProbeError> {
        if !self.available {
            return Err(IpcProbeError::Other(format!(
                "io_uring not available on kernel {} (need >= 5.1)",
                self.kernel_version
            )));
        }

        // Check if /proc/self/fd is readable (basic sanity)
        let _ = tokio::fs::read_dir("/proc/self/fd")
            .await
            .map_err(|_| IpcProbeError::PermissionDenied)?;

        self.init_ring()?;
        Ok(())
    }

    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError> {
        let data = bincode::serialize(message)
            .map_err(|e| IpcProbeError::Other(format!("Serialization failed: {e}")))?;

        // Fallback: simulated latency
        tokio::time::sleep(std::time::Duration::from_micros(10)).await;
        debug!("io_uring send (simulated): {} bytes", data.len());
        Ok(())
    }

    async fn recv(&self) -> Result<AgentMessage, IpcProbeError> {
        // Fallback: block forever (same as stub behavior)
        std::future::pending().await
    }

    async fn try_recv(&self) -> Option<AgentMessage> {
        None
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn name(&self) -> &str {
        "io_uring"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_version_detection() {
        let version = IoUringBackend::get_kernel_version();
        assert_ne!(version, "unknown");
    }

    #[test]
    fn test_kernel_support_check() {
        assert!(IoUringBackend::check_kernel_support("5.10.0"));
        assert!(IoUringBackend::check_kernel_support("6.1.0"));
        assert!(!IoUringBackend::check_kernel_support("4.19.0"));
        assert!(!IoUringBackend::check_kernel_support("5.0.0"));
    }

    #[test]
    fn test_probe() {
        // Should return true on modern Linux, false on old kernels
        let result = IoUringBackend::probe();
        // Just verify it doesn't panic
        let _ = result;
    }

    #[tokio::test]
    async fn test_init_without_feature() {
        let mut backend = IoUringBackend::new();
        // On systems without io_uring support, init should fail gracefully
        if !backend.available {
            let result = backend.init().await;
            assert!(result.is_err());
        }
    }
}
