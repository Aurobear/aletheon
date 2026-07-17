//! Compatibility runtime contract below the canonical G03 AgentControl owner.
//!
//! This module intentionally owns no Agent identity, lifecycle, process,
//! operation, mailbox, lease, or result state. `AgentControlService` is the
//! sole production run authority; these contracts only adapt legacy runtime
//! implementations into its launcher registry.

use async_trait::async_trait;
use fabric::{AttemptUsage, FailureClass, ProcessId, RuntimeFailure, RuntimeResult};
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait SubAgentRuntime: Send + Sync {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>;

    async fn run_attempt(
        &self,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.run(task, cancel)
            .await
            .map(|output| RuntimeResult {
                output,
                usage: AttemptUsage::default(),
                evidence: vec![],
            })
            .map_err(|message| RuntimeFailure {
                class: FailureClass::ToolFailure,
                message,
                retryable: false,
                usage: AttemptUsage::default(),
                evidence: vec![],
            })
    }

    async fn run_in_context(
        &self,
        task: &str,
        cancel: CancellationToken,
        _context: SubAgentExecutionContext,
    ) -> Result<String, String> {
        self.run(task, cancel).await
    }
}

#[derive(Debug, Clone)]
pub struct SubAgentExecutionContext {
    pub process_id: ProcessId,
    pub operation_id: fabric::OperationId,
    pub session_id: String,
    pub working_dir: std::path::PathBuf,
}
