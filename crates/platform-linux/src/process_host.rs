//! Linux ProcessHost — pidfd, process groups, timeout/cancel (H1-02).

use platform_api::error::HostError;
use platform_api::process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
use platform_api::receipt::HostReceipt;
use async_trait::async_trait;

pub struct LinuxProcessHost;

impl LinuxProcessHost {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ProcessHost for LinuxProcessHost {
    async fn spawn(&self, _spec: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError> {
        Err(HostError::unsupported("process spawn"))
    }
    async fn inspect(&self, _id: ProcessId) -> Result<ProcessSnapshot, HostError> {
        Err(HostError::unsupported("process inspect"))
    }
    async fn signal(&self, _id: ProcessId, _signal: ProcessSignal) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("process signal"))
    }
    async fn terminate_tree(&self, _id: ProcessId, _grace_ms: u64) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("process terminate_tree"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn process_host_contract_unimplemented() {
        let host = LinuxProcessHost::new();
        let result = host.spawn(SpawnSpec {
            argv: vec!["true".into()],
            env: vec![],
            working_dir: None,
            timeout_ms: None,
        }).await;
        assert!(result.is_err());
    }
}
