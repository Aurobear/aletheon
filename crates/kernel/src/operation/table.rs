use async_trait::async_trait;
use fabric::{
    CancelReason, Clock, OperationExitReason, OperationHandle, OperationId, OperationManager,
    OperationRecord, OperationRequest, OperationResult, OperationState,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

#[derive(Debug)]
struct OperationRuntime {
    record: OperationRecord,
    notify: Arc<Notify>,
}

/// Authoritative operation tree with parent-cancel propagation.
pub struct OperationTable {
    clock: Arc<dyn Clock>,
    records: Mutex<HashMap<OperationId, OperationRuntime>>,
    children: Mutex<HashMap<OperationId, Vec<OperationId>>>,
}

impl std::fmt::Debug for OperationTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationTable").finish_non_exhaustive()
    }
}

impl OperationTable {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            records: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
        }
    }

    /// Submit a durable operation using its persisted identifier.
    ///
    /// Restart recovery uses this instead of allocating a second operation.
    pub async fn submit_with_id(
        &self,
        id: OperationId,
        req: OperationRequest,
    ) -> anyhow::Result<OperationHandle> {
        let parent = req.parent;
        let owner = req.owner;
        let record = OperationRecord {
            id,
            owner: req.owner,
            parent,
            kind: req.kind,
            state: OperationState::Submitted,
            submitted_at: self.clock.mono_now(),
            deadline: req.deadline,
            exit: None,
        };
        {
            let mut records = self.records.lock().await;
            if records.contains_key(&id) {
                anyhow::bail!("operation {:?} is already registered", id);
            }
            if let Some(parent) = parent {
                let parent_record = &records
                    .get(&parent)
                    .ok_or_else(|| anyhow::anyhow!("unknown parent operation: {parent:?}"))?
                    .record;
                anyhow::ensure!(
                    parent_record.owner == owner,
                    "operation parent belongs to a different process"
                );
                anyhow::ensure!(
                    !parent_record.state.is_terminal(),
                    "operation parent is terminal"
                );
            }
            records.insert(
                id,
                OperationRuntime {
                    record,
                    notify: Arc::new(Notify::new()),
                },
            );
        }
        if let Some(parent) = parent {
            let mut children = self.children.lock().await;
            children.entry(parent).or_default().push(id);
        }
        Ok(OperationHandle { id })
    }

    pub async fn start(&self, id: OperationId) -> anyhow::Result<()> {
        self.set_running_state(id, OperationState::Running, None)
            .await
    }

    pub async fn succeed(&self, id: OperationId) -> anyhow::Result<()> {
        self.set_running_state(
            id,
            OperationState::Succeeded,
            Some(OperationExitReason::Completed),
        )
        .await
    }

    pub async fn fail(&self, id: OperationId, message: impl Into<String>) -> anyhow::Result<()> {
        self.set_running_state(
            id,
            OperationState::Failed,
            Some(OperationExitReason::Failed(message.into())),
        )
        .await
    }

    pub async fn panic(&self, id: OperationId, message: impl Into<String>) -> anyhow::Result<()> {
        self.set_running_state(
            id,
            OperationState::Failed,
            Some(OperationExitReason::Panic(message.into())),
        )
        .await
    }

    pub async fn inspect(&self, id: OperationId) -> anyhow::Result<OperationRecord> {
        let records = self.records.lock().await;
        let runtime = records
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown operation: {:?}", id))?;
        Ok(runtime.record.clone())
    }

    async fn set_running_state(
        &self,
        id: OperationId,
        state: OperationState,
        exit: Option<OperationExitReason>,
    ) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown operation: {:?}", id))?;
            let from = runtime.record.state;
            anyhow::ensure!(
                from.can_transition_to(state),
                "illegal operation transition {from:?} -> {state:?}"
            );
            runtime.record.state = state;
            runtime.record.exit = exit;
            runtime.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    async fn collect_descendants(&self, root: OperationId) -> Vec<OperationId> {
        let children = self.children.lock().await;
        let mut ordered = Vec::new();
        let mut stack = children.get(&root).cloned().unwrap_or_default();
        while let Some(id) = stack.pop() {
            ordered.push(id);
            if let Some(next) = children.get(&id) {
                stack.extend(next.iter().copied());
            }
        }
        ordered
    }

    async fn cancel_one(&self, id: OperationId, reason: CancelReason) -> anyhow::Result<()> {
        let notify = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown operation: {:?}", id))?;
            let from = runtime.record.state;
            if from.is_terminal() {
                return Ok(());
            }
            if from != OperationState::Cancelling {
                anyhow::ensure!(
                    from.can_transition_to(OperationState::Cancelling),
                    "illegal operation transition {from:?} -> Cancelling"
                );
                runtime.record.state = OperationState::Cancelling;
            }
            anyhow::ensure!(
                runtime
                    .record
                    .state
                    .can_transition_to(OperationState::Cancelled),
                "illegal operation transition {:?} -> Cancelled",
                runtime.record.state
            );
            runtime.record.state = OperationState::Cancelled;
            runtime.record.exit = Some(OperationExitReason::Cancelled(reason));
            runtime.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    async fn wait_for_terminal(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        loop {
            let notified = {
                let records = self.records.lock().await;
                let runtime = records
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("unknown operation: {:?}", id))?;
                if runtime.record.state.is_terminal() {
                    return Ok(OperationResult {
                        id,
                        state: runtime.record.state,
                        exit: runtime.record.exit.clone(),
                    });
                }
                runtime.notify.clone().notified_owned()
            };
            notified.await;
        }
    }
}

#[async_trait]
impl OperationManager for OperationTable {
    async fn submit(&self, req: OperationRequest) -> anyhow::Result<OperationHandle> {
        self.submit_with_id(OperationId::new(), req).await
    }

    async fn cancel(&self, id: OperationId, reason: CancelReason) -> anyhow::Result<()> {
        let mut all = self.collect_descendants(id).await;
        all.push(id);
        for op in all {
            self.cancel_one(op, reason.clone()).await?;
        }
        Ok(())
    }

    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        self.wait_for_terminal(id).await
    }
}

impl Default for OperationTable {
    fn default() -> Self {
        Self::new(Arc::new(crate::chronos::SystemClock::new()))
    }
}
