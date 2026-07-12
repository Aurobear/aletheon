use crate::service::{PostTurnPipeline, PreTurnPipeline};
use aletheon_kernel::service_ports::ServicePorts;
use anyhow::Result;
use cognit::harness::{CognitiveSession, HarnessConfig, LinearCognitiveSession};
use fabric::{
    Clock, MonoDeadline, OperationKind, OperationManager, OperationRequest, ProcessManager,
    ProcessSignal, SpawnSpec, TurnEventSink, TurnMetrics, TurnRequest, TurnResult, TurnServices,
    TurnStop,
};
use std::sync::Arc;
use std::time::Duration;

pub struct TurnService {
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
    harness_config: HarnessConfig,
    clock: Arc<dyn Clock>,
    /// Kernel service ports for process/operation lifecycle tracking (PR-2).
    ports: Arc<ServicePorts>,
}

impl TurnService {
    pub fn new(
        services: Arc<dyn TurnServices>,
        pre_turn: PreTurnPipeline,
        post_turn: PostTurnPipeline,
        ports: Arc<ServicePorts>,
    ) -> Self {
        Self {
            services,
            pre_turn,
            post_turn,
            harness_config: HarnessConfig::default(),
            clock: ports.clock.clone(),
            ports,
        }
    }

    pub fn with_harness_config(mut self, harness_config: HarnessConfig) -> Self {
        self.harness_config = harness_config;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub async fn submit(
        &self,
        mut request: TurnRequest,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult> {
        // --- PR-2: register process + operation in kernel tables ---
        // Ensure the calling process exists in the ProcessTable.
        if self
            .ports
            .process_table
            .inspect(request.process_id)
            .await
            .is_err()
        {
            let handle = self
                .ports
                .process_table
                .spawn(SpawnSpec {
                    agent_id: fabric::AgentId::new(),
                    namespace: fabric::NamespaceId(request.session_id.clone()),
                    initial_operation: Some(OperationKind::Turn),
                    ..SpawnSpec::default()
                })
                .await?;
            self.ports
                .process_table
                .signal(handle.id, ProcessSignal::Start)
                .await?;
        }

        // Create a real operation record for this turn.
        let op = self
            .ports
            .operation_table
            .submit(OperationRequest {
                owner: request.process_id,
                parent: None,
                kind: OperationKind::Turn,
                deadline: request
                    .deadline
                    .as_ref()
                    .map(|d| MonoDeadline::after(self.clock.mono_now(), d.0)),
            })
            .await?;
        self.ports.operation_table.start(op.id).await?;
        request.operation_id = op.id;
        // --- end PR-2 kernel wiring ---

        let request = self.pre_turn.run(request, self.services.as_ref()).await?;
        let deadline = request.deadline;
        let mut session = LinearCognitiveSession::new(self.harness_config.clone());
        let start = self.clock.mono_now();

        // TODO(PR-3): replace tokio::time::timeout with Clock::sleep_until
        // for deterministic deadline testing with TestClock.
        let mut result = match deadline {
            Some(deadline_millis) => {
                let deadline_dur = Duration::from_millis(deadline_millis.0);
                match tokio::time::timeout(
                    deadline_dur,
                    session.run_turn(request, self.services.as_ref(), events),
                )
                .await
                {
                    Ok(inner) => inner?,
                    Err(_elapsed) => {
                        self.ports
                            .operation_table
                            .cancel(op.id, fabric::CancelReason::DeadlineExceeded)
                            .await
                            .ok();
                        let elapsed = self.clock.mono_now().0.saturating_sub(start.0);
                        TurnResult {
                            output: String::new(),
                            stop: TurnStop::Cancelled,
                            metrics: TurnMetrics {
                                elapsed_ms: elapsed,
                                completed_normally: false,
                                ..Default::default()
                            },
                        }
                    }
                }
            }
            None => {
                session
                    .run_turn(request, self.services.as_ref(), events)
                    .await?
            }
        };

        // Use clock to compute elapsed time for consistent metrics
        result.metrics.elapsed_ms = self.clock.mono_now().0.saturating_sub(start.0);

        // --- PR-2: settle operation ---
        if result.stop == TurnStop::Cancelled || !result.metrics.completed_normally {
            self.ports
                .operation_table
                .fail(op.id, format!("{:?}", result.stop))
                .await
                .ok();
        } else {
            self.ports.operation_table.succeed(op.id).await.ok();
        }
        // --- end PR-2 ---

        self.post_turn.run(result).await
    }
}
