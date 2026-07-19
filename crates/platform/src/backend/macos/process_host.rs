//! macOS ProcessHost — posix_spawn, process groups (H3).

use crate::error::HostError;
use crate::process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
use crate::receipt::HostReceipt;
use async_trait::async_trait;

pub struct MacOSProcessHost;
impl MacOSProcessHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessHost for MacOSProcessHost {
    async fn spawn(&self, _s: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError> {
        Err(HostError::unsupported("spawn"))
    }
    async fn inspect(&self, _id: ProcessId) -> Result<ProcessSnapshot, HostError> {
        Err(HostError::unsupported("inspect"))
    }
    async fn signal(&self, _id: ProcessId, _sig: ProcessSignal) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("signal"))
    }
    async fn terminate_tree(&self, _id: ProcessId, _ms: u64) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("terminate_tree"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract() {
        assert!(MacOSProcessHost::new()
            .spawn(SpawnSpec {
                argv: vec!["true".into()],
                env: vec![],
                working_dir: None,
                timeout_ms: None
            })
            .await
            .is_err());
    }
}
