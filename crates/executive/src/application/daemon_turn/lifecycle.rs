//! Kernel process-management methods on `DaemonTurnOrchestrator`.
//! TurnPipeline turn-token methods.

use super::orchestrator::DaemonTurnOrchestrator;
use fabric::{
    AgentId, NamespaceId, OperationKind, PrincipalId, ProcessId, ProcessSignal, SpawnSpec, ThreadId,
};
use kernel::supervision::RestartPolicy;
use tokio_util::sync::CancellationToken;
use tracing::info;

impl DaemonTurnOrchestrator {
    /// Ensure the stable root Agent for this principal/thread is registered in
    /// the current Kernel process table. Process IDs are generation-local, but
    /// the root Agent ID survives daemon restarts so durable child runs remain
    /// authorized and discoverable after a session resume.
    pub(crate) async fn ensure_main_agent(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> anyhow::Result<ProcessId> {
        let identity_key = root_identity_key(principal, thread);
        let mut guard = self.main_agent_process_ids.lock().await;
        if let Some(pid) = guard.get(&identity_key).copied() {
            return Ok(pid);
        }
        let root_agent_id = stable_root_agent_id(&identity_key);
        let handle = self
            .kernel
            .spawn_process(SpawnSpec {
                agent_id: root_agent_id,
                namespace: NamespaceId(format!("session:{}", thread.0)),
                initial_operation: Some(OperationKind::Turn),
                ownership: fabric::ProcessOwnership::ThreadBackground {
                    thread_id: thread.clone(),
                },
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
        guard.insert(identity_key, handle.id);
        *self.approval_owner_process_id.lock().await = Some(handle.id);
        info!(process_id = ?handle.id, agent_id = ?root_agent_id, thread_id = %thread.0,
            "Session root agent registered in process table");
        Ok(handle.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_agent_identity_is_stable_and_scoped_to_principal_and_thread() {
        let principal = PrincipalId("local-uid:1000".into());
        let thread = ThreadId("session-a".into());
        let identity = root_identity_key(&principal, &thread);

        assert_eq!(
            stable_root_agent_id(&identity),
            stable_root_agent_id(&identity)
        );
        assert_ne!(
            stable_root_agent_id(&identity),
            stable_root_agent_id(&root_identity_key(
                &principal,
                &ThreadId("session-b".into())
            ))
        );
        assert_ne!(
            stable_root_agent_id(&identity),
            stable_root_agent_id(&root_identity_key(
                &PrincipalId("local-uid:1001".into()),
                &thread
            ))
        );
    }
}

const SESSION_ROOT_AGENT_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x78f4bccc_74f8_49da_98fb_ebd475795cee);

fn root_identity_key(principal: &PrincipalId, thread: &ThreadId) -> String {
    format!("{}\0{}", principal.0, thread.0)
}

fn stable_root_agent_id(identity_key: &str) -> AgentId {
    AgentId(uuid::Uuid::new_v5(
        &SESSION_ROOT_AGENT_NAMESPACE,
        identity_key.as_bytes(),
    ))
}

impl DaemonTurnOrchestrator {
    pub(crate) async fn begin_turn_token(&self) -> CancellationToken {
        let ct = CancellationToken::new();
        let mut token = self.turn_token.lock().await;
        *token = Some(ct.clone());
        ct
    }
}
