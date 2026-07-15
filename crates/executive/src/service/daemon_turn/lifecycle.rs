//! Kernel process-management methods on `DaemonTurnOrchestrator`.
//! TurnPipeline turn-token methods.

use super::orchestrator::DaemonTurnOrchestrator;
use crate::service::turn_pipeline::TurnPipeline;
use aletheon_kernel::supervision::RestartPolicy;
use fabric::{AgentId, NamespaceId, OperationKind, ProcessId, ProcessSignal, SpawnSpec};
use tokio_util::sync::CancellationToken;
use tracing::info;

impl DaemonTurnOrchestrator {
    /// Ensure the main daemon agent is registered in the process table.
    pub(crate) async fn ensure_main_agent(&self) -> anyhow::Result<ProcessId> {
        let mut guard = self.subsystems.main_agent_process_id.lock().await;
        if let Some(pid) = *guard {
            return Ok(pid);
        }
        let handle = self
            .kernel
            .spawn_process(SpawnSpec {
                agent_id: AgentId::new(),
                namespace: NamespaceId("daemon".into()),
                initial_operation: Some(OperationKind::Turn),
                ..SpawnSpec::default()
            })
            .await?;
        self.kernel
            .signal_process(handle.id, ProcessSignal::Start)
            .await?;
        self.kernel
            .supervise(
                handle.id,
                RestartPolicy::RestartOnFailure { max_restarts: 3 },
            )
            .await;
        *guard = Some(handle.id);
        info!(process_id = ?handle.id, "Main daemon agent registered in process table");
        Ok(handle.id)
    }
}

#[allow(dead_code)]
impl TurnPipeline {
    pub(crate) async fn begin_turn_token(&self) -> CancellationToken {
        let ct = CancellationToken::new();
        let mut token = self.subsystems.cancel_token.lock().await;
        *token = Some(ct.clone());
        ct
    }
}
