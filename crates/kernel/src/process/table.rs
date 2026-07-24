use async_trait::async_trait;
use fabric::{
    Clock, ExitReason, ExitStatus, MailboxId, ProcessHandle, ProcessId, ProcessManager,
    ProcessRecord, ProcessSignal, ProcessSnapshot, ProcessState, SpaceId, SpaceManager, SpawnSpec,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

use crate::space::InMemorySpaceManager;

#[derive(Debug)]
struct ProcessRuntime {
    record: ProcessRecord,
    notify: Arc<Notify>,
    active_operation: Option<fabric::OperationId>,
}

/// Authoritative lifecycle table for agent process instances.
pub struct ProcessTable {
    clock: Arc<dyn Clock>,
    records: Mutex<HashMap<ProcessId, ProcessRuntime>>,
    space_manager: Arc<InMemorySpaceManager>,
}

impl std::fmt::Debug for ProcessTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessTable").finish_non_exhaustive()
    }
}

impl ProcessTable {
    pub async fn connection_foreground_ids(
        &self,
        connection_id: &fabric::ConnectionId,
    ) -> Vec<ProcessId> {
        self.records
            .lock()
            .await
            .iter()
            .filter_map(|(id, runtime)| {
                matches!(
                    &runtime.record.ownership,
                    fabric::ProcessOwnership::ConnectionForeground { connection_id: owner }
                        if owner == connection_id
                )
                .then_some(*id)
            })
            .collect()
    }
    /// Create a table with its own private space manager (tests, standalone).
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_space_manager(clock, Arc::new(InMemorySpaceManager::new()))
    }

    /// Create a table sharing a space manager with the rest of the kernel, so
    /// spawn/exit can fork and release context spaces.
    pub fn with_space_manager(
        clock: Arc<dyn Clock>,
        space_manager: Arc<InMemorySpaceManager>,
    ) -> Self {
        Self {
            clock,
            records: Mutex::new(HashMap::new()),
            space_manager,
        }
    }

    pub async fn transition(&self, id: ProcessId, next: ProcessState) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
            let from = runtime.record.state;
            if !from.can_transition_to(next) {
                anyhow::bail!("illegal process transition {from:?} -> {next:?}");
            }
            runtime.record.state = next;
            runtime.record.last_heartbeat = self.clock.mono_now();
            runtime.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    pub async fn set_active_operation(
        &self,
        id: ProcessId,
        operation: Option<fabric::OperationId>,
    ) -> anyhow::Result<()> {
        let mut records = self.records.lock().await;
        let runtime = records
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
        runtime.active_operation = operation;
        Ok(())
    }

    pub async fn mark_exit(&self, id: ProcessId, reason: ExitReason) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
            let from = runtime.record.state;
            anyhow::ensure!(!from.is_terminal(), "process {id:?} is already terminal");
            let terminal = match &reason {
                ExitReason::Failed(_) | ExitReason::Panic(_) => ProcessState::Failed,
                _ => ProcessState::Exited,
            };
            if terminal == ProcessState::Exited && from != ProcessState::Stopping {
                anyhow::ensure!(
                    from.can_transition_to(ProcessState::Stopping),
                    "illegal process exit transition {from:?} -> Stopping"
                );
                runtime.record.state = ProcessState::Stopping;
            }
            anyhow::ensure!(
                runtime.record.state.can_transition_to(terminal),
                "illegal process exit transition {:?} -> {terminal:?}",
                runtime.record.state
            );
            runtime.record.exit = Some(ExitStatus {
                reason: reason.clone(),
                at: self.clock.mono_now(),
            });
            runtime.record.state = terminal;
            runtime.record.last_heartbeat = self.clock.mono_now();
            runtime.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    pub async fn reap(&self, id: ProcessId) -> anyhow::Result<ProcessRecord> {
        let mut records = self.records.lock().await;
        let runtime = records
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
        if !runtime.record.state.is_terminal() {
            anyhow::bail!("cannot reap non-terminal process {id:?}");
        }
        Ok(records.remove(&id).expect("checked above").record)
    }

    async fn wait_for_terminal(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        loop {
            let notified = {
                let records = self.records.lock().await;
                let runtime = records
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
                if let Some(exit) = &runtime.record.exit {
                    return Ok(exit.clone());
                }
                runtime.notify.clone().notified_owned()
            };
            notified.await;
        }
    }

    fn snapshot(runtime: &ProcessRuntime) -> ProcessSnapshot {
        ProcessSnapshot {
            process_id: runtime.record.process_id,
            space: runtime.record.space,
            agent_id: runtime.record.agent_id,
            parent: runtime.record.parent,
            profile: runtime.record.profile.clone(),
            state: runtime.record.state,
            exit: runtime.record.exit.clone(),
            active_operation: runtime.active_operation,
            ownership: runtime.record.ownership.clone(),
        }
    }
}

#[async_trait]
impl ProcessManager for ProcessTable {
    async fn spawn(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        let process_id = ProcessId::new();
        // Look up the parent's space (scoped lock — released before await).
        let parent_space = {
            let records = self.records.lock().await;
            match spec.parent {
                Some(parent) => {
                    let runtime = records
                        .get(&parent)
                        .ok_or_else(|| anyhow::anyhow!("unknown parent process: {parent:?}"))?;
                    anyhow::ensure!(
                        !runtime.record.state.is_terminal(),
                        "parent process is terminal"
                    );
                    Some(runtime.record.space)
                }
                None => None,
            }
        };
        // Fork the child space from the parent (inherits bindings read-only),
        // or mint a fresh root space for parentless processes.
        let space = match parent_space {
            Some(parent_space) => {
                self.space_manager
                    .fork_space(parent_space, process_id)
                    .await?
            }
            None => SpaceId::new(),
        };
        let record = ProcessRecord {
            process_id,
            agent_id: spec.agent_id,
            parent: spec.parent,
            profile: spec.profile,
            state: ProcessState::Created,
            space,
            mailbox: MailboxId::new(),
            namespace: spec.namespace,
            created_at: self.clock.wall_now(),
            last_heartbeat: self.clock.mono_now(),
            exit: None,
            ownership: spec.ownership,
        };
        let mut records = self.records.lock().await;
        records.insert(
            process_id,
            ProcessRuntime {
                record,
                notify: Arc::new(Notify::new()),
                active_operation: None,
            },
        );
        Ok(ProcessHandle { id: process_id })
    }

    async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> anyhow::Result<()> {
        match signal {
            ProcessSignal::Start => {
                self.transition(id, ProcessState::Ready).await?;
                self.transition(id, ProcessState::Running).await
            }
            ProcessSignal::Wait => self.transition(id, ProcessState::Waiting).await,
            ProcessSignal::Resume => self.transition(id, ProcessState::Running).await,
            ProcessSignal::Terminate => {
                let state = self.inspect(id).await?.state;
                match state {
                    ProcessState::Created => {
                        self.transition(id, ProcessState::Ready).await?;
                        self.transition(id, ProcessState::Running).await?;
                        self.transition(id, ProcessState::Stopping).await?;
                    }
                    ProcessState::Ready => {
                        self.transition(id, ProcessState::Running).await?;
                        self.transition(id, ProcessState::Stopping).await?;
                    }
                    ProcessState::Running | ProcessState::Waiting => {
                        self.transition(id, ProcessState::Stopping).await?;
                    }
                    ProcessState::Stopping => {}
                    ProcessState::Exited | ProcessState::Failed => return Ok(()),
                }
                self.mark_exit(id, ExitReason::Cancelled("terminated".into()))
                    .await
            }
            ProcessSignal::Kill => {
                if self.inspect(id).await?.state.is_terminal() {
                    Ok(())
                } else {
                    self.mark_exit(id, ExitReason::Panic("killed".into())).await
                }
            }
        }
    }

    async fn wait(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        self.wait_for_terminal(id).await
    }

    async fn inspect(&self, id: ProcessId) -> anyhow::Result<ProcessSnapshot> {
        let records = self.records.lock().await;
        let runtime = records
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown process: {id:?}"))?;
        Ok(Self::snapshot(runtime))
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new(Arc::new(crate::chronos::SystemClock::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::process::SpawnSpec;

    #[tokio::test]
    async fn snapshot_exposes_process_space() {
        let table = ProcessTable::default();
        let h1 = table.spawn(SpawnSpec::default()).await.unwrap();
        let h2 = table.spawn(SpawnSpec::default()).await.unwrap();
        let s1 = table.inspect(h1.id).await.unwrap();
        let s1_again = table.inspect(h1.id).await.unwrap();
        let s2 = table.inspect(h2.id).await.unwrap();
        assert_eq!(s1.space, s1_again.space, "space stable per process");
        assert_ne!(s1.space, s2.space, "each spawn mints a unique space");
    }

    #[tokio::test]
    async fn spawn_forks_child_space_from_parent() {
        use crate::chronos::SystemClock;
        use fabric::types::space::{ContextBinding, SessionId};
        let sm = std::sync::Arc::new(InMemorySpaceManager::new());
        let table =
            ProcessTable::with_space_manager(std::sync::Arc::new(SystemClock::new()), sm.clone());
        let parent = table.spawn(SpawnSpec::default()).await.unwrap();
        let parent_space = table.inspect(parent.id).await.unwrap().space;
        sm.upsert_binding(parent_space, ContextBinding::Session(SessionId("s".into())));
        let child = table
            .spawn(SpawnSpec {
                parent: Some(parent.id),
                ..SpawnSpec::default()
            })
            .await
            .unwrap();
        let child_space = table.inspect(child.id).await.unwrap().space;
        assert_ne!(child_space, parent_space, "child gets its own space");
        let cb = sm.get_bindings(child_space).unwrap();
        assert!(
            cb.iter().any(|b| matches!(b, ContextBinding::Session(_))),
            "inherited parent binding"
        );
    }

    #[tokio::test]
    async fn table_terminal_transition_defers_cross_resource_cleanup_to_runtime() {
        use crate::chronos::SystemClock;
        use fabric::types::space::{ContextBinding, SessionId};
        let sm = std::sync::Arc::new(InMemorySpaceManager::new());
        let table =
            ProcessTable::with_space_manager(std::sync::Arc::new(SystemClock::new()), sm.clone());
        let h = table.spawn(SpawnSpec::default()).await.unwrap();
        let space = table.inspect(h.id).await.unwrap().space;
        sm.upsert_binding(space, ContextBinding::Session(SessionId("s".into())));
        assert_eq!(sm.space_count(), 1);
        table
            .signal(h.id, fabric::types::process::ProcessSignal::Terminate)
            .await
            .unwrap();
        assert!(
            sm.get_space(space).is_some(),
            "opaque KernelRuntime, not the state table, owns resource cleanup"
        );
    }
}
