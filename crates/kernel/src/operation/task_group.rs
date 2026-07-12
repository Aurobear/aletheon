use fabric::{OperationExitReason, OperationId};
use std::future::Future;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Structured task exit recorded by an operation scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExit {
    pub name: String,
    pub reason: OperationExitReason,
}

/// Structured concurrency scope for tasks owned by one Operation.
pub struct OperationScope {
    pub id: OperationId,
    pub cancel: CancellationToken,
    pub tasks: JoinSet<TaskExit>,
}

impl std::fmt::Debug for OperationScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationScope")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl OperationScope {
    pub fn new(id: OperationId) -> Self {
        Self {
            id,
            cancel: CancellationToken::new(),
            tasks: JoinSet::new(),
        }
    }

    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn spawn<F>(&mut self, name: impl Into<String>, fut: F)
    where
        F: Future<Output = OperationExitReason> + Send + 'static,
    {
        let name = name.into();
        self.tasks.spawn(async move {
            let reason = fut.await;
            TaskExit { name, reason }
        });
    }

    pub async fn join_next(&mut self) -> Option<TaskExit> {
        match self.tasks.join_next().await? {
            Ok(exit) => Some(exit),
            Err(err) => Some(TaskExit {
                name: "<join-error>".into(),
                reason: if err.is_panic() {
                    OperationExitReason::Panic(format!("{err}"))
                } else {
                    OperationExitReason::Cancelled(fabric::CancelReason::Other(format!("{err}")))
                },
            }),
        }
    }

    pub async fn cancel_and_drain(&mut self, grace: Duration) -> Vec<TaskExit> {
        self.cancel.cancel();
        let mut exits = Vec::new();
        loop {
            if self.tasks.is_empty() {
                break;
            }
            match tokio::time::timeout(grace, self.join_next()).await {
                Ok(Some(exit)) => exits.push(exit),
                Ok(None) => break,
                Err(_) => {
                    self.tasks.abort_all();
                    while let Some(exit) = self.join_next().await {
                        exits.push(exit);
                    }
                    break;
                }
            }
        }
        exits
    }
}
