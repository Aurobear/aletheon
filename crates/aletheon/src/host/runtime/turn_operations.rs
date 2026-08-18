use std::sync::Arc;

use application::turn::ports::{TurnOperationMetrics, TurnOperationPort, TurnTimerPort};
use async_trait::async_trait;
use kernel::OperationManager;

pub struct KernelTurnOperations {
    kernel: Arc<kernel::KernelRuntime>,
}

impl KernelTurnOperations {
    pub fn new(kernel: Arc<kernel::KernelRuntime>) -> Self {
        Self { kernel }
    }
}

#[async_trait]
impl TurnOperationPort for KernelTurnOperations {
    async fn admit(
        &self,
        owner: contracts::ProcessId,
        deadline: Option<contracts::MonoDeadline>,
    ) -> anyhow::Result<contracts::OperationId> {
        let operation = self
            .kernel
            .submit(contracts::OperationRequest {
                owner,
                parent: None,
                kind: contracts::OperationKind::Turn,
                deadline,
            })
            .await?;
        self.kernel.start_operation(operation.id).await?;
        Ok(operation.id)
    }

    async fn succeed(&self, operation: contracts::OperationId) -> anyhow::Result<()> {
        self.kernel
            .succeed_operation(operation)
            .await
            .map_err(Into::into)
    }

    async fn fail(&self, operation: contracts::OperationId, reason: String) -> anyhow::Result<()> {
        self.kernel
            .fail_operation(operation, reason)
            .await
            .map_err(Into::into)
    }

    async fn cancel(
        &self,
        operation: contracts::OperationId,
        reason: contracts::CancelReason,
    ) -> anyhow::Result<()> {
        self.kernel
            .cancel(operation, reason)
            .await
            .map_err(Into::into)
    }

    fn metrics(&self) -> TurnOperationMetrics {
        let metrics = kernel::operation::operation_scope_metrics();
        TurnOperationMetrics {
            active_scopes: metrics.active_scopes,
            active_resources: metrics.active_resources,
            drop_fallbacks: metrics.drop_fallbacks,
            pending_reclaim: metrics.pending_reclaim,
            reclaim_queue_exhausted: metrics.reclaim_queue_exhausted,
        }
    }
}

pub enum KernelTurnTimer {
    System(kernel::chronos::SystemTimer),
    Test(Arc<kernel::chronos::TestTimer>),
}

impl KernelTurnTimer {
    pub fn system() -> Self {
        Self::System(kernel::chronos::SystemTimer)
    }

    pub fn test(timer: kernel::chronos::TestTimer) -> Self {
        Self::Test(Arc::new(timer))
    }

    pub fn shared_test(timer: Arc<kernel::chronos::TestTimer>) -> Self {
        Self::Test(timer)
    }
}

#[async_trait]
impl TurnTimerPort for KernelTurnTimer {
    async fn sleep(&self, duration: std::time::Duration) {
        use contracts::Timer as _;
        match self {
            Self::System(timer) => timer.sleep(duration).await,
            Self::Test(timer) => timer.sleep(duration).await,
        }
    }
}
