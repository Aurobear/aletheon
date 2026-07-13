//! Kernel process-management methods on `DaemonTurnOrchestrator`.

use super::orchestrator::DaemonTurnOrchestrator;
use aletheon_kernel::supervision::RestartPolicy;
use fabric::{
    AgentId, NamespaceId, OperationKind, ProcessId, ProcessManager, ProcessSignal, SpawnSpec,
};
use tracing::info;

impl DaemonTurnOrchestrator {
    /// Ensure the main daemon agent is registered in the process table.
    pub(crate) async fn ensure_main_agent(&self) -> anyhow::Result<ProcessId> {
        let mut guard = self.subsystems.main_agent_process_id.lock().await;
        if let Some(pid) = *guard {
            return Ok(pid);
        }
        let handle = self
            .process_table
            .spawn(SpawnSpec {
                agent_id: AgentId::new(),
                namespace: NamespaceId("daemon".into()),
                initial_operation: Some(OperationKind::SubAgent),
                ..SpawnSpec::default()
            })
            .await?;
        self.process_table
            .signal(handle.id, ProcessSignal::Start)
            .await?;
        self.supervisor.lock().await.supervise(
            handle.id,
            RestartPolicy::RestartOnFailure { max_restarts: 3 },
        );
        *guard = Some(handle.id);
        info!(process_id = ?handle.id, "Main daemon agent registered in process table");
        Ok(handle.id)
    }
}
