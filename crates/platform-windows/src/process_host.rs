//! Windows ProcessHost — Job Objects, CreateProcessW, ConPTY (H2).

use platform_api::error::HostError;
use platform_api::process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
use platform_api::receipt::HostReceipt;
use async_trait::async_trait;

pub struct WindowsProcessHost;
impl WindowsProcessHost { pub fn new() -> Self { Self } }

#[async_trait]
impl ProcessHost for WindowsProcessHost {
    async fn spawn(&self, _spec: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError> {
        Err(HostError::unsupported("process spawn — CreateProcessW + Job Object not wired"))
    }
    async fn inspect(&self, _id: ProcessId) -> Result<ProcessSnapshot, HostError> {
        Err(HostError::unsupported("process inspect"))
    }
    async fn signal(&self, _id: ProcessId, _signal: ProcessSignal) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("process signal"))
    }
    async fn terminate_tree(&self, _id: ProcessId, _grace_ms: u64) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("process terminate_tree — Job kill-on-close not wired"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test] async fn contract_unimplemented() { assert!(WindowsProcessHost::new().spawn(SpawnSpec{argv:vec!["cmd".into()],env:vec![],working_dir:None,timeout_ms:None}).await.is_err()); }
}
