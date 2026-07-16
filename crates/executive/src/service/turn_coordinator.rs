//! Single owner of Turn operations and canonical lifecycle settlement.

use std::{collections::HashMap, future::Future, sync::Arc};

use aletheon_kernel::KernelRuntime;
use anyhow::{anyhow, Result};
use fabric::{
    CancelReason, EventSpine, ItemId, ItemPayload, ItemRecord, MonoDeadline, OperationKind,
    OperationManager, OperationRequest, SessionAppendStore, SessionId, SessionRecord,
    SessionStatus, TurnId, TurnMetrics, TurnRequest, TurnResult, TurnStop, SESSION_SCHEMA_VERSION,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::turn_policy::TurnPolicy;

pub struct TurnExecution {
    pub result: TurnResult,
    pub items: Vec<ItemPayload>,
    pub projection: Option<super::post_turn_projection::PostTurnDispatch>,
    pub context_projection: Option<fabric::ContextProjectionReceipt>,
}

struct CompletedExecution {
    result: TurnResult,
    projection: Option<super::post_turn_projection::PostTurnDispatch>,
}

#[derive(Clone)]
pub struct ActiveTurn {
    pub operation_id: fabric::OperationId,
    pub cancel: CancellationToken,
}

pub struct TurnCoordinator {
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn fabric::Clock>,
    read_store: Arc<dyn SessionAppendStore>,
    store: Arc<dyn SessionAppendStore>,
    event_spine: Arc<dyn EventSpine>,
    active: Arc<Mutex<HashMap<String, ActiveTurn>>>,
}

impl TurnCoordinator {
    pub fn new(kernel: Arc<KernelRuntime>, store: Arc<dyn SessionAppendStore>) -> Self {
        let event_spine = Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(":memory:")
                .expect("in-memory event spine"),
        );
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components(kernel, store, event_spine, projections)
    }

    pub fn with_event_spine(
        kernel: Arc<KernelRuntime>,
        store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
    ) -> Self {
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components(kernel, store, event_spine, projections)
    }

    fn with_components(
        kernel: Arc<KernelRuntime>,
        read_store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
        projections: Arc<dyn super::event_projection::EventProjectionSink>,
    ) -> Self {
        let store: Arc<dyn SessionAppendStore> = Arc::new(
            crate::r#impl::session::event_sourced_store::EventSourcedSessionStore::new(
                read_store.clone(),
                event_spine.clone(),
                projections,
            ),
        );
        Self {
            clock: kernel.clock(),
            kernel,
            read_store,
            store,
            event_spine,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_event_projections(
        mut self,
        projections: Arc<dyn super::event_projection::EventProjectionSink>,
    ) -> Self {
        self.store = Arc::new(
            crate::r#impl::session::event_sourced_store::EventSourcedSessionStore::new(
                self.read_store.clone(),
                self.event_spine.clone(),
                projections,
            ),
        );
        self
    }

    pub fn store(&self) -> Arc<dyn SessionAppendStore> {
        self.store.clone()
    }
    pub fn active_index(&self) -> Arc<Mutex<HashMap<String, ActiveTurn>>> {
        self.active.clone()
    }

    pub async fn cancel_operation(&self, operation_id: fabric::OperationId) -> bool {
        let active = self.active.lock().await;
        if let Some(turn) = active
            .values()
            .find(|turn| turn.operation_id == operation_id)
        {
            turn.cancel.cancel();
            true
        } else {
            false
        }
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
            .kernel
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
        self.kernel.start_operation(operation.id).await?;
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
            Ok(completed) => {
                let terminal = match completed.result.stop {
                    TurnStop::Completed if completed.result.metrics.completed_normally => {
                        self.kernel.succeed_operation(operation.id).await
                    }
                    TurnStop::Cancelled => {
                        self.kernel
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
                        self.kernel
                            .fail_operation(operation.id, format!("{:?}", completed.result.stop))
                            .await
                    }
                };
                terminal?;
                if let Some(dispatch) = completed.projection {
                    tokio::spawn(async move {
                        if let Err(error) = dispatch.projector.project(dispatch.outcome).await {
                            tracing::warn!(%error, "post-turn projection failed after settlement");
                        }
                    });
                }
                Ok(completed.result)
            }
            Err(error) => {
                self.kernel
                    .fail_operation(operation.id, error.to_string())
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
    ) -> Result<CompletedExecution>
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
                let TurnExecution {
                    result,
                    items,
                    projection,
                    context_projection,
                } = execution;
                if let Some(receipt) = context_projection {
                    receipt.validate()?;
                    self.append(
                        &session_id,
                        turn_id,
                        &mut sequence,
                        ItemPayload::ContextProjection {
                            space: receipt.space.0,
                            broadcast_epoch: receipt.broadcast_epoch.map(|epoch| epoch.0),
                            workspace_version: receipt.workspace_version,
                            dasein_version: receipt.dasein_version.0,
                            content_ids: receipt
                                .content_ids
                                .into_iter()
                                .map(|id| id.0.to_string())
                                .collect(),
                        },
                    )
                    .await?;
                }
                for payload in items {
                    self.append(&session_id, turn_id, &mut sequence, payload)
                        .await?;
                }
                let terminal = if result.stop == TurnStop::Completed {
                    ItemPayload::AssistantMessage {
                        content: result.output.clone(),
                    }
                } else {
                    ItemPayload::SystemNotice {
                        content: format!("turn stopped: {:?}", result.stop),
                    }
                };
                self.append(&session_id, turn_id, &mut sequence, terminal)
                    .await?;
                Ok(CompletedExecution { result, projection })
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
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id: session.clone(),
            turn_id,
            sequence: current,
            created_at_ms: self.now_ms(),
            payload,
        };
        self.store.append(session, current, item).await?;
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
