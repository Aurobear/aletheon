//! Compatibility adapter from the public `CognitiveSession` boundary to the
//! new append-only Harness core.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use contracts::{
    CapabilityCall, ContentBlock, LlmProvider, Message, Role, TurnEvent, TurnEventSink,
    TurnMetrics, TurnRequest, TurnResult, TurnServices, TurnStop,
};
use tokio_util::sync::CancellationToken;

use super::agent::{AgentInbox, AgentSendRequest};
use super::driver::{
    HarnessDriverError, HarnessLoopDriver, HarnessLoopPorts, HarnessModelRequest,
    HarnessModelResponse, HarnessModelService, HarnessPortError, HarnessPromptAssembler,
    HarnessStepInputSource, HarnessToolExecutionMode, HarnessToolInvocation, HarnessToolOutput,
    HarnessToolService,
};
use super::lifecycle::{HarnessLifecycleHooks, NoopHarnessLifecycleHooks};
use super::session::{
    invoke_with_terminal_receipt, CognitError, CognitiveSession, ProjectionRecordingLlm,
};
use super::session_log::{
    HarnessRequestHeader, HarnessSessionEventKind, HarnessSessionId, HarnessSessionLog,
    HarnessSessionPersistence, TurnEndReason,
};

struct TurnPrompt(String);

#[async_trait]
impl HarnessPromptAssembler for TurnPrompt {
    async fn system_prompt(&self) -> Result<String, HarnessPortError> {
        Ok(self.0.clone())
    }
}

struct TurnModel<'a> {
    provider: &'a dyn LlmProvider,
    seed: Vec<Message>,
    base_surface_len: usize,
    seed_contains_turn_input: bool,
}

#[async_trait]
impl HarnessModelService for TurnModel<'_> {
    async fn infer(
        &self,
        request: HarnessModelRequest,
        cancellation: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError> {
        if cancellation.is_cancelled() {
            return Err(HarnessPortError {
                code: "cancelled".into(),
                message: "cancelled before provider dispatch".into(),
            });
        }
        let mut messages = Vec::with_capacity(self.seed.len() + request.messages.len() + 1);
        if !request.system_prompt.is_empty() {
            messages.push(Message::system(request.system_prompt));
        }
        messages.extend(self.seed.iter().cloned());
        let mut turn_messages = request.messages.into_iter().skip(self.base_surface_len);
        if self.seed_contains_turn_input {
            let _ = turn_messages.next();
        }
        messages.extend(turn_messages);
        let response = self
            .provider
            .complete(&messages, &request.tool_definitions)
            .await
            .map_err(|error| HarnessPortError {
                code: "provider_error".into(),
                message: error.to_string(),
            })?;
        Ok(HarnessModelResponse {
            chunks: vec![],
            message: Message {
                role: Role::Assistant,
                content: response.content,
            },
            usage: response.usage,
            stop_reason: response.stop_reason,
            provider_retries: 0,
            active_context_tokens: None,
        })
    }
}

struct TurnTools<'a> {
    services: &'a dyn TurnServices,
    request: &'a TurnRequest,
    clock: &'a dyn contracts::Clock,
    definitions: tokio::sync::Mutex<Vec<contracts::ToolDefinition>>,
    batch_planner: Option<Arc<dyn super::linear::BatchPlanner>>,
}

#[async_trait]
impl HarnessToolService for TurnTools<'_> {
    async fn definitions(&self) -> Result<Vec<contracts::ToolDefinition>, HarnessPortError> {
        let activated = self.services.drain_activated_tool_definitions().await;
        let mut definitions = self.definitions.lock().await;
        for definition in activated {
            if !definitions
                .iter()
                .any(|visible| visible.name == definition.name)
            {
                definitions.push(definition);
            }
        }
        Ok(definitions.clone())
    }

    async fn plan(
        &self,
        invocations: Vec<HarnessToolInvocation>,
    ) -> Result<Vec<HarnessToolInvocation>, HarnessPortError> {
        let calls = invocations
            .iter()
            .map(|invocation| CapabilityCall {
                operation_id: self.request.operation_id,
                process_id: self.request.process_id,
                name: invocation.name.clone(),
                input: invocation.input.clone(),
                call_id: invocation.call_id.clone(),
                deadline: None,
            })
            .collect::<Vec<_>>();
        let plan = match &self.batch_planner {
            Some(planner) => planner.plan(calls.clone()).await,
            None => self.services.plan_capability_batch(calls.clone()).await,
        }
        .map_err(|error| HarnessPortError {
            code: "capability_batch_plan_failed".into(),
            message: error.to_string(),
        })?;
        if plan.mode != contracts::ConsciousArbitrationMode::Enforce {
            return Ok(invocations);
        }
        if let Err(error) = plan.validate_against(&calls) {
            tracing::warn!(%error, "invalid enforced capability batch plan; preserving provider order");
            return Ok(invocations);
        }
        let by_id = invocations
            .into_iter()
            .map(|invocation| (invocation.call_id.clone(), invocation))
            .collect::<std::collections::HashMap<_, _>>();
        Ok(plan
            .ordered_call_ids
            .into_iter()
            .filter_map(|call_id| by_id.get(&call_id).cloned())
            .collect())
    }

    fn execution_mode(&self, _name: &str) -> HarnessToolExecutionMode {
        // The compatibility boundary has no synchronous capability metadata;
        // serial execution is safe until composition injects a typed classifier.
        HarnessToolExecutionMode::Exclusive
    }

    async fn execute(
        &self,
        invocation: HarnessToolInvocation,
        _cancellation: CancellationToken,
    ) -> HarnessToolOutput {
        let result = invoke_with_terminal_receipt(
            self.services,
            CapabilityCall {
                operation_id: self.request.operation_id,
                process_id: self.request.process_id,
                name: invocation.name,
                input: invocation.input,
                call_id: invocation.call_id,
                deadline: None,
            },
            self.clock,
        )
        .await;
        HarnessToolOutput {
            content: result.output,
            is_error: result.is_error,
            error_code: result.is_error.then(|| "CAPABILITY_ERROR".into()),
        }
    }
}

struct TurnInputs<'a> {
    services: &'a dyn TurnServices,
}

#[async_trait]
impl HarnessStepInputSource for TurnInputs<'_> {
    async fn drain(&self) -> Result<Vec<Message>, HarnessPortError> {
        self.services
            .drain_interjections()
            .await
            .map(|messages| messages.into_iter().map(Message::user).collect())
            .map_err(|error| HarnessPortError {
                code: "step_input_failed".into(),
                message: error.to_string(),
            })
    }
}

pub struct HarnessCognitiveSession {
    session: Arc<Mutex<HarnessSessionLog>>,
    inbox: Arc<AgentInbox>,
    clock: Arc<dyn contracts::Clock>,
    cancellation: CancellationToken,
    hooks: Arc<dyn HarnessLifecycleHooks>,
    batch_planner: Option<Arc<dyn super::linear::BatchPlanner>>,
}

impl HarnessCognitiveSession {
    pub fn new(
        id: HarnessSessionId,
        clock: Arc<dyn contracts::Clock>,
        cancellation: CancellationToken,
    ) -> Result<Self, CognitError> {
        Self::with_hooks_and_persistence(
            id,
            clock,
            cancellation,
            Arc::new(NoopHarnessLifecycleHooks),
            None,
        )
    }

    pub fn with_hooks_and_persistence(
        id: HarnessSessionId,
        clock: Arc<dyn contracts::Clock>,
        cancellation: CancellationToken,
        hooks: Arc<dyn HarnessLifecycleHooks>,
        persistence: Option<Arc<dyn HarnessSessionPersistence>>,
    ) -> Result<Self, CognitError> {
        let log = match persistence {
            Some(persistence) => HarnessSessionLog::new_persistent(id, persistence),
            None => HarnessSessionLog::new(id),
        }
        .map_err(|error| CognitError::terminal(error.to_string()))?;
        let session = Arc::new(Mutex::new(log));
        let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
        Ok(Self {
            session,
            inbox,
            clock,
            cancellation,
            hooks,
            batch_planner: None,
        })
    }

    pub fn with_batch_planner(mut self, planner: Arc<dyn super::linear::BatchPlanner>) -> Self {
        self.batch_planner = Some(planner);
        self
    }

    pub fn event_log(&self) -> Arc<Mutex<HarnessSessionLog>> {
        self.session.clone()
    }
}

#[async_trait]
impl CognitiveSession for HarnessCognitiveSession {
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
        let Some(provider) = services.llm_provider() else {
            let result = passthrough_result(request.input.clone());
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: result.stop.clone(),
                })
                .await;
            return Ok(result);
        };

        let turn = self
            .session
            .lock()
            .map_err(|_| CognitError::terminal("harness session lock poisoned"))?
            .next_turn_number();
        self.inbox
            .send(AgentSendRequest::followup(
                format!("turn-{turn}"),
                Message::user(request.input.clone()),
            ))
            .map_err(|error| CognitError::terminal(error.to_string()))?;

        let seed = services.seed_messages(&request);
        let seed_contains_turn_input = !request.input.is_empty()
            && seed
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .is_some_and(|message| message_text(message).contains(&request.input));
        let prompt = TurnPrompt(String::new());
        let base_surface_len = self
            .session
            .lock()
            .map_err(|_| CognitError::terminal("harness session lock poisoned"))?
            .derive_messages()
            .len();
        let recording = ProjectionRecordingLlm::new(provider, services, request.operation_id);
        let model = TurnModel {
            provider: &recording,
            seed,
            base_surface_len,
            seed_contains_turn_input,
        };
        let inputs = TurnInputs { services };
        let tools = TurnTools {
            services,
            request: &request,
            clock: self.clock.as_ref(),
            definitions: tokio::sync::Mutex::new(services.tool_definitions()),
            batch_planner: self.batch_planner.clone(),
        };
        let facts = provider.runtime_facts();
        let driver = HarnessLoopDriver::new(
            self.session.clone(),
            self.inbox.clone(),
            HarnessLoopPorts {
                prompt: &prompt,
                model: &model,
                inputs: &inputs,
                tools: &tools,
                hooks: self.hooks.as_ref(),
                clock: self.clock.clone(),
            },
        );
        let core_result = driver
            .run_turn(
                HarnessRequestHeader {
                    provider: facts.provider_id.unwrap_or_else(|| provider.name().into()),
                    model: facts.effective_model_id,
                    reasoning_effort: None,
                    max_output_tokens: None,
                },
                self.cancellation.clone(),
            )
            .await;
        let result = match core_result {
            Ok(result) => {
                let (stop, completed_normally) = terminal_stop(&self.session, turn)?;
                if self.cancellation.is_cancelled() {
                    events
                        .emit(TurnEvent::Finished {
                            operation_id: request.operation_id,
                            stop: TurnStop::Cancelled,
                        })
                        .await;
                    return Err(CognitError::cancelled());
                }
                let output = result
                    .final_message
                    .as_ref()
                    .map(message_text)
                    .unwrap_or_default();
                let tool_errors = count_tool_errors(&self.session, turn)?;
                TurnResult {
                    output,
                    stop: stop.clone(),
                    failure: None,
                    usage: result.cumulative_usage,
                    metrics: TurnMetrics {
                        tool_calls_made: result.tool_calls as usize,
                        tool_errors,
                        provider_retries: result.provider_retries,
                        elapsed_ms: 0,
                        iterations: result.inference_rounds as usize,
                        completed_normally,
                    },
                }
            }
            Err(error) => {
                events
                    .emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Failed,
                    })
                    .await;
                return Err(map_driver_error(error));
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

fn message_text(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn terminal_stop(
    session: &Arc<Mutex<HarnessSessionLog>>,
    turn: u64,
) -> Result<(TurnStop, bool), CognitError> {
    let log = session
        .lock()
        .map_err(|_| CognitError::terminal("harness session lock poisoned"))?;
    let reason = log
        .events()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            HarnessSessionEventKind::TurnEnd {
                turn: event_turn,
                reason,
            } if *event_turn == turn => Some(reason),
            _ => None,
        });
    Ok(match reason {
        Some(TurnEndReason::Completed) => (TurnStop::Completed, true),
        Some(TurnEndReason::MaxTokens) => (TurnStop::Completed, false),
        Some(TurnEndReason::Blocked) => (TurnStop::Blocked, false),
        Some(TurnEndReason::Aborted { .. }) => (TurnStop::Cancelled, false),
        Some(TurnEndReason::Error { .. }) | None => (TurnStop::Failed, false),
    })
}

fn count_tool_errors(
    session: &Arc<Mutex<HarnessSessionLog>>,
    turn: u64,
) -> Result<usize, CognitError> {
    Ok(session
        .lock()
        .map_err(|_| CognitError::terminal("harness session lock poisoned"))?
        .events()
        .iter()
        .filter(|event| matches!(event.kind, HarnessSessionEventKind::ToolResult { turn: event_turn, is_error: true, .. } if event_turn == turn))
        .count())
}

fn map_driver_error(error: HarnessDriverError) -> CognitError {
    match error {
        HarnessDriverError::Port(port) => CognitError::from_runtime(anyhow::anyhow!(port)),
        other => CognitError::terminal(other.to_string()),
    }
}

fn passthrough_result(output: String) -> TurnResult {
    TurnResult {
        output,
        stop: TurnStop::Completed,
        failure: None,
        usage: Default::default(),
        metrics: TurnMetrics {
            completed_normally: true,
            ..Default::default()
        },
    }
}
