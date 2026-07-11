use crate::kernel::chronos::SystemClock;
use crate::service::{PostTurnPipeline, PreTurnPipeline};
use anyhow::Result;
use cognit::harness::{CognitiveSession, HarnessConfig, LinearCognitiveSession};
use fabric::{Clock, TurnEventSink, TurnMetrics, TurnRequest, TurnResult, TurnServices, TurnStop};
use std::sync::Arc;
use std::time::Duration;

pub struct TurnService {
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
    harness_config: HarnessConfig,
    clock: Arc<dyn Clock>,
}

impl TurnService {
    pub fn new(
        services: Arc<dyn TurnServices>,
        pre_turn: PreTurnPipeline,
        post_turn: PostTurnPipeline,
    ) -> Self {
        Self {
            services,
            pre_turn,
            post_turn,
            harness_config: HarnessConfig::default(),
            clock: Arc::new(SystemClock::new()),
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
        request: TurnRequest,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult> {
        let request = self.pre_turn.run(request, self.services.as_ref()).await?;
        let deadline = request.deadline;
        let mut session = LinearCognitiveSession::new(self.harness_config.clone());
        let start = self.clock.mono_now();

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

        self.post_turn.run(result).await
    }
}
