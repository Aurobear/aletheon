//! Object-safe cognitive session adapter used by the executive turn service.

use crate::harness::config::HarnessConfig;
use crate::harness::linear::DynLlmRef;
use crate::harness::linear::{CompactorTrait, ReActLoop};
use anyhow::Result;
use async_trait::async_trait;
use fabric::{
    CapabilityRequest, Message, TurnEvent, TurnEventSink, TurnMetrics as FabricTurnMetrics,
    TurnRequest, TurnResult, TurnServices, TurnStop,
};
use std::pin::Pin;

struct NoopCompressor;

impl CompactorTrait for NoopCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn crate::r#impl::llm::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn crate::r#impl::llm::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

#[async_trait]
pub trait CognitiveSession: Send {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult>;
}

pub struct LinearCognitiveSession {
    inner: ReActLoop,
}

impl LinearCognitiveSession {
    pub fn new(config: HarnessConfig) -> Self {
        Self {
            inner: ReActLoop::new(config, Box::new(NoopCompressor)),
        }
    }

    /// Create a session wrapping a pre-built ReActLoop.
    ///
    /// Useful when the loop is constructed by a shared factory, e.g.
    /// `harness_factory::build_configured_react_loop()` in the daemon path.
    pub fn from_react_loop(inner: ReActLoop) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl CognitiveSession for LinearCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;

        let result = if let Some(llm) = services.llm_provider() {
            self.inner.reset();
            let seed_messages = services.seed_messages(&request);
            if !seed_messages.is_empty() {
                self.inner.seed_messages(seed_messages);
            }
            let tool_defs = services.tool_definitions();
            let process_id = request.process_id;
            let (output, metrics) = self
                .inner
                .run(
                    &request.input,
                    &DynLlmRef(llm),
                    &tool_defs,
                    |call_id, name, input| {
                        let req = CapabilityRequest {
                            operation_id: request.operation_id,
                            process_id,
                            name: name.to_string(),
                            input: input.clone(),
                            call_id: call_id.to_string(),
                            deadline: None,
                        };
                        async move {
                            let result = services.invoke(req).await;
                            (result.output, result.is_error)
                        }
                    },
                )
                .await?;
            TurnResult {
                output,
                stop: TurnStop::Completed,
                metrics: FabricTurnMetrics {
                    tool_calls_made: metrics.tool_calls_made,
                    tool_errors: metrics.tool_errors,
                    elapsed_ms: metrics.elapsed_ms,
                    iterations: metrics.iterations,
                    completed_normally: metrics.completed_normally,
                },
            }
        } else {
            TurnResult {
                output: request.input,
                stop: TurnStop::Completed,
                metrics: FabricTurnMetrics {
                    completed_normally: true,
                    ..Default::default()
                },
            }
        };

        events
            .emit(TurnEvent::Finished {
                operation_id: request.operation_id,
                stop: result.stop.clone(),
            })
            .await;
        Ok(result)
    }
}
