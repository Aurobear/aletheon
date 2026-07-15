//! Single owner of Turn operations and canonical lifecycle settlement.

use std::{collections::HashMap, future::Future, sync::Arc};

use aletheon_kernel::service::ServicePorts;
use anyhow::{anyhow, Result};
use fabric::{
    CancelReason, ItemId, ItemPayload, ItemRecord, MonoDeadline, OperationKind, OperationManager,
    OperationRequest, SessionAppendStore, SessionId, SessionRecord, SessionStatus, TurnId,
    TurnMetrics, TurnRequest, TurnResult, TurnStop, SESSION_SCHEMA_VERSION,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::turn_policy::TurnPolicy;

pub struct TurnExecution {
    pub result: TurnResult,
    pub items: Vec<ItemPayload>,
}

#[derive(Clone)]
pub struct ActiveTurn {
    pub operation_id: fabric::OperationId,
    pub cancel: CancellationToken,
}

pub struct TurnCoordinator {
    operations: Arc<aletheon_kernel::operation::OperationTable>,
    clock: Arc<dyn fabric::Clock>,
    store: Arc<dyn SessionAppendStore>,
    active: Arc<Mutex<HashMap<String, ActiveTurn>>>,
}

impl TurnCoordinator {
    pub fn new(ports: &ServicePorts, store: Arc<dyn SessionAppendStore>) -> Self {
        Self {
            operations: ports.operation_table.clone(),
            clock: ports.clock.clone(),
            store,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn store(&self) -> Arc<dyn SessionAppendStore> {
        self.store.clone()
    }
    pub fn active_index(&self) -> Arc<Mutex<HashMap<String, ActiveTurn>>> {
        self.active.clone()
    }

    pub async fn submit_with<F, Fut>(
        &self,
        mut request: TurnRequest,
        _policy: &TurnPolicy,
        runner: F,
    ) -> Result<TurnResult>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>>,
    {
        let operation = self
            .operations
            .submit(OperationRequest {
                owner: request.process_id,
                parent: None,
                kind: OperationKind::Turn,
                deadline: request
                    .deadline
                    .map(|d| MonoDeadline::after(self.clock.mono_now(), d.0)),
            })
            .await?;
        let has_deadline = request.deadline.is_some();
        self.operations.start(operation.id).await?;
        request.operation_id = operation.id;
        let cancel = CancellationToken::new();
        self.active.lock().await.insert(
            request.session_id.clone(),
            ActiveTurn {
                operation_id: operation.id,
                cancel: cancel.clone(),
            },
        );

        let outcome = self
            .run_started_turn(&request, cancel.clone(), runner)
            .await;
        self.active.lock().await.remove(&request.session_id);
        match outcome {
            Ok(result) => {
                let terminal = match result.stop {
                    TurnStop::Completed if result.metrics.completed_normally => {
                        self.operations.succeed(operation.id).await
                    }
                    TurnStop::Cancelled => {
                        self.operations
                            .cancel(
                                operation.id,
                                if has_deadline {
                                    CancelReason::DeadlineExceeded
                                } else {
                                    CancelReason::User
                                },
                            )
                            .await
                    }
                    _ => {
                        self.operations
                            .fail(operation.id, format!("{:?}", result.stop))
                            .await
                    }
                };
                terminal?;
                Ok(result)
            }
            Err(error) => {
                self.operations
                    .fail(operation.id, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    async fn run_started_turn<F, Fut>(
        &self,
        request: &TurnRequest,
        cancel: CancellationToken,
        runner: F,
    ) -> Result<TurnResult>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>>,
    {
        let session_id = SessionId(request.session_id.clone());
        if self.store.load_session(&session_id).await?.is_none() {
            self.store
                .create(SessionRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: session_id.clone(),
                    parent: None,
                    created_at_ms: self.now_ms(),
                    status: SessionStatus::Active,
                })
                .await?;
        }
        let turn_id = TurnId::new();
        let mut sequence = self.next_sequence(&session_id).await?;
        self.append(
            &session_id,
            turn_id,
            &mut sequence,
            ItemPayload::UserMessage {
                content: request.input.clone(),
            },
        )
        .await?;

        let execution = runner(request.clone(), cancel).await;
        match execution {
            Ok(execution) => {
                for payload in execution.items {
                    self.append(&session_id, turn_id, &mut sequence, payload)
                        .await?;
                }
                let terminal = if execution.result.stop == TurnStop::Completed {
                    ItemPayload::AssistantMessage {
                        content: execution.result.output.clone(),
                    }
                } else {
                    ItemPayload::SystemNotice {
                        content: format!("turn stopped: {:?}", execution.result.stop),
                    }
                };
                self.append(&session_id, turn_id, &mut sequence, terminal)
                    .await?;
                Ok(execution.result)
            }
            Err(error) => {
                self.append(
                    &session_id,
                    turn_id,
                    &mut sequence,
                    ItemPayload::SystemNotice {
                        content: format!("turn failed: {error}"),
                    },
                )
                .await
                .map_err(|append| {
                    anyhow!("turn failed: {error}; failure notice append failed: {append}")
                })?;
                Err(error)
            }
        }
    }

    async fn next_sequence(&self, session: &SessionId) -> Result<u64> {
        Ok(self
            .store
            .load_items(session, None)
            .await?
            .last()
            .map_or(1, |item| item.sequence + 1))
    }

    async fn append(
        &self,
        session: &SessionId,
        turn_id: TurnId,
        sequence: &mut u64,
        payload: ItemPayload,
    ) -> Result<()> {
        let current = *sequence;
        self.store
            .append(
                session,
                current,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session.clone(),
                    turn_id,
                    sequence: current,
                    created_at_ms: self.now_ms(),
                    payload,
                },
            )
            .await?;
        *sequence += 1;
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        self.clock.wall_now().0.max(0) as u64
    }
}

pub fn cancelled_result() -> TurnResult {
    TurnResult {
        output: String::new(),
        stop: TurnStop::Cancelled,
        metrics: TurnMetrics {
            completed_normally: false,
            ..Default::default()
        },
    }
}
