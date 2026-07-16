//! Object-safe cognitive session adapter used by the executive turn service.

use crate::harness::config::HarnessConfig;
use crate::harness::linear::DynLlmRef;
use crate::harness::linear::{CompactorTrait, ReActLoop};
use async_trait::async_trait;
use fabric::{
    CapabilityCall, Message, TurnEvent, TurnEventSink, TurnMetrics as FabricTurnMetrics,
    TurnRequest, TurnResult, TurnServices, TurnStop,
};
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub type CognitiveStreamEvent = crate::harness::event_sink::Event;

/// Cognit-owned streaming event boundary used by interactive frontends.
pub trait CognitiveStreamSink: Send + Sync {
    fn emit(&self, event: CognitiveStreamEvent);
}

pub struct ChannelCognitiveStreamSink {
    tx: tokio::sync::mpsc::Sender<CognitiveStreamEvent>,
}

impl ChannelCognitiveStreamSink {
    pub fn new(tx: tokio::sync::mpsc::Sender<CognitiveStreamEvent>) -> Self {
        Self { tx }
    }
}

impl CognitiveStreamSink for ChannelCognitiveStreamSink {
    fn emit(&self, event: CognitiveStreamEvent) {
        let _ = self.tx.try_send(event);
    }
}

struct CognitiveStreamAdapter<'a>(&'a dyn CognitiveStreamSink);

impl crate::harness::event_sink::EventSink for CognitiveStreamAdapter<'_> {
    fn emit(&self, event: CognitiveStreamEvent) {
        self.0.emit(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CognitRetryDisposition {
    Never,
    AfterBackoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CognitErrorKind {
    Cancelled,
    ContextOverflow,
    TransientProvider,
    TerminalRuntime,
}

#[derive(Debug, Error)]
#[error("cognitive session {kind:?}: {message}")]
pub struct CognitError {
    kind: CognitErrorKind,
    message: String,
}

impl CognitError {
    pub fn kind(&self) -> CognitErrorKind {
        self.kind
    }

    pub const fn retry_disposition(&self) -> CognitRetryDisposition {
        match self.kind {
            CognitErrorKind::TransientProvider => CognitRetryDisposition::AfterBackoff,
            CognitErrorKind::Cancelled
            | CognitErrorKind::ContextOverflow
            | CognitErrorKind::TerminalRuntime => CognitRetryDisposition::Never,
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: CognitErrorKind::Cancelled,
            message: "turn cancellation requested".into(),
        }
    }

    fn from_runtime(error: anyhow::Error) -> Self {
        use crate::r#impl::llm::scheduler::{classify_error, ErrorClass};
        let kind = match classify_error(&error) {
            ErrorClass::Transient => CognitErrorKind::TransientProvider,
            ErrorClass::ContextOverflow => CognitErrorKind::ContextOverflow,
            ErrorClass::Terminal => CognitErrorKind::TerminalRuntime,
        };
        Self {
            kind,
            message: bounded_error(&error.to_string()),
        }
    }
}

pub struct CognitiveSessionDependencies {
    pub clock: Arc<dyn fabric::Clock>,
    pub cancellation: CancellationToken,
    pub compactor: Option<Box<dyn CompactorTrait>>,
}

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
    ) -> Result<TurnResult, CognitError>;

    async fn run_streaming_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
        stream: &dyn CognitiveStreamSink,
    ) -> Result<TurnResult, CognitError> {
        let result = self.run_turn(request, services, events).await?;
        stream.emit(CognitiveStreamEvent::Text {
            text: result.output.clone(),
        });
        stream.emit(CognitiveStreamEvent::TurnDone {
            result: Ok(result.output.clone()),
        });
        Ok(result)
    }
}

pub trait CognitiveSessionFactory: Send + Sync {
    fn create(
        &self,
        config: HarnessConfig,
        dependencies: CognitiveSessionDependencies,
    ) -> Result<Box<dyn CognitiveSession>, CognitError>;
}

#[derive(Default)]
pub struct DefaultCognitiveSessionFactory;

impl CognitiveSessionFactory for DefaultCognitiveSessionFactory {
    fn create(
        &self,
        config: HarnessConfig,
        dependencies: CognitiveSessionDependencies,
    ) -> Result<Box<dyn CognitiveSession>, CognitError> {
        Ok(Box::new(LinearCognitiveSession::new(config, dependencies)))
    }
}

pub struct LinearCognitiveSession {
    inner: ReActLoop,
    cancellation: CancellationToken,
}

impl LinearCognitiveSession {
    pub fn new(config: HarnessConfig, dependencies: CognitiveSessionDependencies) -> Self {
        let compactor = dependencies
            .compactor
            .unwrap_or_else(|| Box::new(NoopCompressor));
        Self {
            inner: ReActLoop::new_with_clock(config, compactor, dependencies.clock),
            cancellation: dependencies.cancellation,
        }
    }

    /// Create a session wrapping a pre-built ReActLoop.
    ///
    /// Useful when the loop is constructed by a shared factory, e.g.
    /// `harness_factory::build_configured_react_loop()` in the daemon path.
    pub fn from_react_loop(inner: ReActLoop, cancellation: CancellationToken) -> Self {
        Self {
            inner,
            cancellation,
        }
    }
}

#[async_trait]
impl CognitiveSession for LinearCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;

        if self.cancellation.is_cancelled() {
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                })
                .await;
            return Err(CognitError::cancelled());
        }

        let result = if let Some(llm) = services.llm_provider() {
            self.inner.reset();
            let seed_messages = services.seed_messages(&request);
            if !seed_messages.is_empty() {
                self.inner.seed_messages(seed_messages);
            }
            let tool_defs = services.tool_definitions();
            let process_id = request.process_id;
            let llm = DynLlmRef(llm);
            let run = self
                .inner
                .run(&request.input, &llm, &tool_defs, |call_id, name, input| {
                    let req = CapabilityCall {
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
                });
            let (output, metrics) = tokio::select! {
                _ = self.cancellation.cancelled() => {
                    events.emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Cancelled,
                    }).await;
                    return Err(CognitError::cancelled());
                }
                result = run => match result {
                    Ok(result) => result,
                    Err(error) => {
                        events.emit(TurnEvent::Finished {
                            operation_id: request.operation_id,
                            stop: TurnStop::Failed,
                        }).await;
                        return Err(CognitError::from_runtime(error));
                    }
                }
            };
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

    async fn run_streaming_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
        stream: &dyn CognitiveStreamSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;
        if self.cancellation.is_cancelled() {
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                })
                .await;
            return Err(CognitError::cancelled());
        }

        let Some(llm) = services.llm_provider() else {
            let result = TurnResult {
                output: request.input,
                stop: TurnStop::Completed,
                metrics: FabricTurnMetrics {
                    completed_normally: true,
                    ..Default::default()
                },
            };
            stream.emit(CognitiveStreamEvent::Text {
                text: result.output.clone(),
            });
            stream.emit(CognitiveStreamEvent::TurnDone {
                result: Ok(result.output.clone()),
            });
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: result.stop.clone(),
                })
                .await;
            return Ok(result);
        };

        self.inner.reset();
        let seed_messages = services.seed_messages(&request);
        if !seed_messages.is_empty() {
            self.inner.seed_messages(seed_messages);
        }
        self.inner.set_goal(request.input.clone());
        if let Ok(view) = services.dasein_view(request.process_id).await {
            self.inner
                .set_dasein_context_provider(Box::new(move || view.text.clone()));
        }
        stream.emit(CognitiveStreamEvent::GoalSet {
            goal: request.input,
            sub_goals: vec![],
        });

        let tool_defs = services.tool_definitions();
        let process_id = request.process_id;
        let llm = DynLlmRef(llm);
        let sink = CognitiveStreamAdapter(stream);
        let run = self.inner.run_streaming(
            &llm,
            &tool_defs,
            |call_id, name, input| {
                let call = CapabilityCall {
                    operation_id: request.operation_id,
                    process_id,
                    name: name.to_string(),
                    input: input.clone(),
                    call_id: call_id.to_string(),
                    deadline: None,
                };
                async move {
                    let result = services.invoke(call).await;
                    (result.output, result.is_error)
                }
            },
            &sink,
        );
        let (output, metrics) = tokio::select! {
            _ = self.cancellation.cancelled() => {
                events.emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                }).await;
                return Err(CognitError::cancelled());
            }
            result = run => match result {
                Ok(result) => result,
                Err(error) => {
                    events.emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Failed,
                    }).await;
                    return Err(CognitError::from_runtime(error));
                }
            }
        };
        let result = TurnResult {
            output,
            stop: TurnStop::Completed,
            metrics: FabricTurnMetrics {
                tool_calls_made: metrics.tool_calls_made,
                tool_errors: metrics.tool_errors,
                elapsed_ms: metrics.elapsed_ms,
                iterations: metrics.iterations,
                completed_normally: metrics.completed_normally,
            },
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

fn bounded_error(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}
