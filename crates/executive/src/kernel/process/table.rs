use async_trait::async_trait;
use fabric::{
    Clock, ExitReason, ExitStatus, MailboxId, ProcessHandle, ProcessId, ProcessManager,
    ProcessRecord, ProcessSignal, ProcessSnapshot, ProcessState, SpaceId, SpawnSpec,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

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
}

impl std::fmt::Debug for ProcessTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessTable").finish_non_exhaustive()
    }
}

impl ProcessTable {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub async fn transition(&self, id: ProcessId, next: ProcessState) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
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
            .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
        runtime.active_operation = operation;
        Ok(())
    }

    pub async fn mark_exit(&self, id: ProcessId, reason: ExitReason) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
            runtime.record.exit = Some(ExitStatus {
                reason: reason.clone(),
                at: self.clock.mono_now(),
            });
            runtime.record.state = match reason {
                ExitReason::Failed(_) | ExitReason::Panic(_) => ProcessState::Failed,
                _ => ProcessState::Exited,
            };
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
            .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
        if !runtime.record.state.is_terminal() {
            anyhow::bail!("cannot reap non-terminal process {:?}", id);
        }
        Ok(records.remove(&id).expect("checked above").record)
    }

    async fn wait_for_terminal(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        loop {
            let notified = {
                let records = self.records.lock().await;
                let runtime = records
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
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
            agent_id: runtime.record.agent_id,
            parent: runtime.record.parent,
            profile: runtime.record.profile.clone(),
            state: runtime.record.state,
            exit: runtime.record.exit.clone(),
            active_operation: runtime.active_operation,
        }
    }
}

#[async_trait]
impl ProcessManager for ProcessTable {
    async fn spawn(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        let process_id = ProcessId::new();
        let record = ProcessRecord {
            process_id,
            agent_id: spec.agent_id,
            parent: spec.parent,
            profile: spec.profile,
            state: ProcessState::Created,
            space: SpaceId::new(),
            mailbox: MailboxId::new(),
            namespace: spec.namespace,
            created_at: self.clock.wall_now(),
            last_heartbeat: self.clock.mono_now(),
            exit: None,
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
                    ProcessState::Stopping | ProcessState::Exited | ProcessState::Failed => {}
                }
                self.mark_exit(id, ExitReason::Cancelled("terminated".into()))
                    .await
            }
            ProcessSignal::Kill => self.mark_exit(id, ExitReason::Panic("killed".into())).await,
        }
    }

    async fn wait(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        self.wait_for_terminal(id).await
    }

    async fn inspect(&self, id: ProcessId) -> anyhow::Result<ProcessSnapshot> {
        let records = self.records.lock().await;
        let runtime = records
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
        Ok(Self::snapshot(runtime))
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new(Arc::new(crate::kernel::chronos::SystemClock::new()))
    }
}
