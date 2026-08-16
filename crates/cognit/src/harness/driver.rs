//! Minimal model -> tools -> repeat Harness driver.
//!
//! Lifecycle policy (budgets, compaction, verification and goal handling) is
//! deliberately absent. Those concerns wrap this driver instead of becoming
//! branches in the loop.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use contracts::{
    canonicalize_tool_definitions, Clock, ContentBlock, InferenceUsage, Message, StopReason,
    StreamChunk, ToolDefinition,
};
use futures::future::join_all;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::agent::AgentInbox;
use super::lifecycle::{
    HarnessLifecycleHooks, HarnessPreStepDecision, HarnessRequestErrorDecision, HarnessStepContext,
    HarnessToolPreDecision, HarnessTurnStoppingDecision,
};
use super::session_log::{
    HarnessRequestHeader, HarnessSessionEventKind, HarnessSessionLog, HarnessSessionLogError,
    RequestHeaderReason, SurfaceOp, TurnEndReason,
};

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessModelRequest {
    pub header: HarnessRequestHeader,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub tool_definitions: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessModelResponse {
    pub chunks: Vec<StreamChunk>,
    pub message: Message,
    pub usage: InferenceUsage,
    pub stop_reason: StopReason,
    pub provider_retries: u64,
    pub active_context_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessToolInvocation {
    pub call_id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessToolOutput {
    pub content: String,
    pub is_error: bool,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessToolExecutionMode {
    Parallel,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct HarnessPortError {
    pub code: String,
    pub message: String,
}

#[async_trait]
pub trait HarnessPromptAssembler: Send + Sync {
    async fn system_prompt(&self) -> Result<String, HarnessPortError>;
}

#[async_trait]
pub trait HarnessModelService: Send + Sync {
    async fn infer(
        &self,
        request: HarnessModelRequest,
        cancellation: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError>;
}

#[async_trait]
pub trait HarnessStepInputSource: Send + Sync {
    async fn drain(&self) -> Result<Vec<Message>, HarnessPortError> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Default)]
pub struct NoopHarnessStepInputSource;

impl HarnessStepInputSource for NoopHarnessStepInputSource {}

#[async_trait]
pub trait HarnessToolService: Send + Sync {
    async fn definitions(&self) -> Result<Vec<ToolDefinition>, HarnessPortError>;
    async fn plan(
        &self,
        invocations: Vec<HarnessToolInvocation>,
    ) -> Result<Vec<HarnessToolInvocation>, HarnessPortError> {
        Ok(invocations)
    }
    fn execution_mode(&self, name: &str) -> HarnessToolExecutionMode;
    async fn execute(
        &self,
        invocation: HarnessToolInvocation,
        cancellation: CancellationToken,
    ) -> HarnessToolOutput;
}

#[derive(Clone)]
pub struct HarnessLoopPorts<'a> {
    pub prompt: &'a dyn HarnessPromptAssembler,
    pub model: &'a dyn HarnessModelService,
    pub inputs: &'a dyn HarnessStepInputSource,
    pub tools: &'a dyn HarnessToolService,
    pub hooks: &'a dyn HarnessLifecycleHooks,
    pub clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessTurnResult {
    pub final_message: Option<Message>,
    pub inference_rounds: u64,
    pub provider_retries: u64,
    pub tool_calls: u64,
    pub active_context_tokens: Option<u64>,
    pub cumulative_usage: InferenceUsage,
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessDriverError {
    #[error(transparent)]
    Session(#[from] HarnessSessionLogError),
    #[error("inbox: {0}")]
    Inbox(String),
    #[error("{0}")]
    Port(#[from] HarnessPortError),
    #[error("session log lock is poisoned")]
    Poisoned,
    #[error("model response must contain one assistant message")]
    InvalidAssistantMessage,
    #[error("tool arguments could not be serialized: {0}")]
    ToolArguments(String),
}

pub struct HarnessLoopDriver<'a> {
    session: Arc<Mutex<HarnessSessionLog>>,
    inbox: Arc<AgentInbox>,
    ports: HarnessLoopPorts<'a>,
    turn_gate: AsyncMutex<()>,
}

impl<'a> HarnessLoopDriver<'a> {
    pub fn new(
        session: Arc<Mutex<HarnessSessionLog>>,
        inbox: Arc<AgentInbox>,
        ports: HarnessLoopPorts<'a>,
    ) -> Self {
        Self {
            session,
            inbox,
            ports,
            turn_gate: AsyncMutex::new(()),
        }
    }

    pub async fn run_turn(
        &self,
        header: HarnessRequestHeader,
        cancellation: CancellationToken,
    ) -> Result<HarnessTurnResult, HarnessDriverError> {
        let _guard = self.turn_gate.lock().await;
        let turn = self
            .session
            .lock()
            .map_err(|_| HarnessDriverError::Poisoned)?
            .next_turn_number();
        self.append(HarnessSessionEventKind::TurnStart { turn }, None, vec![])?;
        let first = match self.inbox.claim_for_turn() {
            Ok(claim) => claim,
            Err(error) => {
                let error = HarnessDriverError::Inbox(error.to_string());
                self.close_turn_error(turn, &error)?;
                return Err(error);
            }
        };
        if first.messages.is_empty() {
            self.append(
                HarnessSessionEventKind::TurnEnd {
                    turn,
                    reason: TurnEndReason::Blocked,
                },
                None,
                vec![],
            )?;
            return Ok(HarnessTurnResult {
                final_message: None,
                inference_rounds: 0,
                provider_retries: 0,
                tool_calls: 0,
                active_context_tokens: None,
                cumulative_usage: InferenceUsage::default(),
            });
        }

        let mut incoming = first.messages;
        let mut usages = Vec::new();
        let mut rounds = 0;
        let mut retries = 0;
        let mut tool_calls = 0;
        let mut active_context_tokens = None;
        let mut final_message = None;

        loop {
            let step = self
                .session
                .lock()
                .map_err(|_| HarnessDriverError::Poisoned)?
                .next_step_number();
            self.append(
                HarnessSessionEventKind::StepStart { turn, step },
                None,
                vec![],
            )?;
            for item in incoming.drain(..) {
                self.append(
                    HarnessSessionEventKind::UserMessage {
                        turn,
                        step,
                        source: item.source,
                        message: item.message,
                    },
                    Some(SurfaceOp::Append),
                    vec![],
                )?;
            }
            let context = HarnessStepContext { turn, step };
            match self.ports.hooks.pre_step(context).await {
                Ok(HarnessPreStepDecision::Continue) => {}
                Ok(HarnessPreStepDecision::End(reason)) => {
                    self.close_step_and_turn(turn, step, reason)?;
                    return Ok(Self::result(
                        final_message,
                        rounds,
                        retries,
                        tool_calls,
                        active_context_tokens,
                        &usages,
                    ));
                }
                Err(error) => {
                    let error = HarnessDriverError::Port(error);
                    self.close_step_error(turn, step, &error)?;
                    return Err(error);
                }
            }
            if cancellation.is_cancelled() {
                self.close_step_and_turn(
                    turn,
                    step,
                    TurnEndReason::Aborted {
                        cause: "cancelled_before_model_dispatch".into(),
                    },
                )?;
                return Ok(Self::result(
                    final_message,
                    rounds,
                    retries,
                    tool_calls,
                    active_context_tokens,
                    &usages,
                ));
            }

            let mut system_prompt = match self.ports.prompt.system_prompt().await {
                Ok(prompt) => prompt,
                Err(error) => {
                    let error = HarnessDriverError::Port(error);
                    self.close_step_error(turn, step, &error)?;
                    return Err(error);
                }
            };
            let raw_definitions = match self.ports.tools.definitions().await {
                Ok(definitions) => definitions,
                Err(error) => {
                    let error = HarnessDriverError::Port(error);
                    self.close_step_error(turn, step, &error)?;
                    return Err(error);
                }
            };
            let mut definitions =
                match canonicalize_tool_definitions(&raw_definitions).map_err(|e| {
                    HarnessPortError {
                        code: "invalid_tool_definitions".into(),
                        message: e.to_string(),
                    }
                }) {
                    Ok(definitions) => definitions,
                    Err(error) => {
                        let error = HarnessDriverError::Port(error);
                        self.close_step_error(turn, step, &error)?;
                        return Err(error);
                    }
                };
            if let Err(error) = self
                .ports
                .hooks
                .prepare_request(context, &mut system_prompt, &mut definitions)
                .await
            {
                let error = HarnessDriverError::Port(error);
                self.close_step_error(turn, step, &error)?;
                return Err(error);
            }
            definitions = match canonicalize_tool_definitions(&definitions) {
                Ok(definitions) => definitions,
                Err(error) => {
                    let error = HarnessDriverError::Port(HarnessPortError {
                        code: "invalid_hook_tool_definitions".into(),
                        message: error.to_string(),
                    });
                    self.close_step_error(turn, step, &error)?;
                    return Err(error);
                }
            };
            self.append(
                HarnessSessionEventKind::RequestHeader {
                    turn,
                    step,
                    header: header.clone(),
                    reason: if rounds == 0 {
                        RequestHeaderReason::Initial
                    } else {
                        RequestHeaderReason::Resume
                    },
                },
                None,
                vec![],
            )?;
            let surface = self
                .session
                .lock()
                .map_err(|_| HarnessDriverError::Poisoned)?
                .surface_seqs()
                .to_vec();
            self.append(
                HarnessSessionEventKind::RequestPrepared {
                    turn,
                    step,
                    system_prompt: system_prompt.clone(),
                    tool_definitions: definitions.clone(),
                },
                None,
                surface,
            )?;
            let messages = self
                .session
                .lock()
                .map_err(|_| HarnessDriverError::Poisoned)?
                .derive_messages();
            let request = HarnessModelRequest {
                header: header.clone(),
                system_prompt,
                messages,
                tool_definitions: definitions,
            };
            let mut attempt = 1_u32;
            let response = loop {
                if cancellation.is_cancelled() {
                    self.close_step_and_turn(
                        turn,
                        step,
                        TurnEndReason::Aborted {
                            cause: "cancelled_before_model_dispatch".into(),
                        },
                    )?;
                    return Ok(Self::result(
                        final_message,
                        rounds,
                        retries,
                        tool_calls,
                        active_context_tokens,
                        &usages,
                    ));
                }
                let response = {
                    let infer = self
                        .ports
                        .model
                        .infer(request.clone(), cancellation.clone());
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                            self.close_step_and_turn(
                                turn,
                                step,
                                TurnEndReason::Aborted {
                                    cause: "cancelled_during_model_dispatch".into(),
                                },
                            )?;
                            return Ok(Self::result(
                                final_message,
                                rounds,
                                retries,
                                tool_calls,
                                active_context_tokens,
                                &usages,
                            ));
                        }
                        result = infer => result,
                    }
                };
                match response {
                    Ok(response) => break response,
                    Err(error) => {
                        let decision = self
                            .ports
                            .hooks
                            .request_error(context, attempt, &error)
                            .await;
                        let will_retry = decision == HarnessRequestErrorDecision::Retry;
                        self.append(
                            HarnessSessionEventKind::RequestError {
                                turn,
                                step,
                                attempt,
                                code: error.code.clone(),
                                message: error.message.clone(),
                                will_retry,
                            },
                            None,
                            vec![],
                        )?;
                        if will_retry {
                            retries = retries.saturating_add(1);
                            attempt = attempt.saturating_add(1);
                            continue;
                        }
                        let error = HarnessDriverError::Port(error);
                        self.close_step_error(turn, step, &error)?;
                        return Err(error);
                    }
                }
            };
            if response.message.role != contracts::Role::Assistant {
                let error = HarnessDriverError::InvalidAssistantMessage;
                self.close_step_error(turn, step, &error)?;
                return Err(error);
            }
            rounds += 1;
            retries += response.provider_retries;
            active_context_tokens = response.active_context_tokens;
            usages.push(response.usage.clone());
            let mut chunk_seqs = Vec::new();
            for chunk in response.chunks {
                chunk_seqs.push(self.append(
                    HarnessSessionEventKind::AssistantChunk { turn, step, chunk },
                    None,
                    vec![],
                )?);
            }
            self.append(
                HarnessSessionEventKind::AssistantMessage {
                    turn,
                    step,
                    message: response.message.clone(),
                    usage: response.usage,
                },
                Some(SurfaceOp::Append),
                chunk_seqs,
            )?;
            final_message = Some(response.message.clone());

            if response.stop_reason == StopReason::MaxTokens {
                self.close_step_and_turn(turn, step, TurnEndReason::MaxTokens)?;
                return Ok(Self::result(
                    final_message,
                    rounds,
                    retries,
                    tool_calls,
                    active_context_tokens,
                    &usages,
                ));
            }

            let mut calls = Vec::new();
            for block in response.message.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    calls.push(HarnessToolInvocation {
                        call_id: id,
                        name,
                        input,
                    });
                }
            }
            tool_calls += calls.len() as u64;
            if calls.is_empty() {
                self.append(
                    HarnessSessionEventKind::StepEnd { turn, step },
                    None,
                    vec![],
                )?;
                incoming = self.claim_next_step(turn, step).await?;
                if !incoming.is_empty() {
                    continue;
                }
                match self.ports.hooks.turn_stopping(context).await {
                    Ok(HarnessTurnStoppingDecision::Stop) => {
                        self.append(
                            HarnessSessionEventKind::TurnEnd {
                                turn,
                                reason: TurnEndReason::Completed,
                            },
                            None,
                            vec![],
                        )?;
                        return Ok(Self::result(
                            final_message,
                            rounds,
                            retries,
                            tool_calls,
                            active_context_tokens,
                            &usages,
                        ));
                    }
                    Ok(HarnessTurnStoppingDecision::Continue) => {
                        continue;
                    }
                    Err(error) => {
                        let error = HarnessDriverError::Port(error);
                        self.close_step_error(turn, step, &error)?;
                        return Err(error);
                    }
                }
            }

            let calls = match self.ports.tools.plan(calls).await {
                Ok(calls) => calls,
                Err(error) => {
                    let error = HarnessDriverError::Port(error);
                    self.close_step_error(turn, step, &error)?;
                    return Err(error);
                }
            };
            let calls = calls
                .into_iter()
                .map(|invocation| {
                    let arguments = serde_json::to_string(&invocation.input)
                        .map_err(|error| HarnessDriverError::ToolArguments(error.to_string()))?;
                    let seq = self.append(
                        HarnessSessionEventKind::ToolCall {
                            turn,
                            step,
                            call_id: invocation.call_id.clone(),
                            name: invocation.name.clone(),
                            arguments,
                        },
                        None,
                        vec![],
                    )?;
                    Ok((invocation, seq))
                })
                .collect::<Result<Vec<_>, HarnessDriverError>>()?;
            self.execute_calls(turn, step, calls, cancellation.clone())
                .await?;
            self.append(
                HarnessSessionEventKind::StepEnd { turn, step },
                None,
                vec![],
            )?;
            incoming = self.claim_next_step(turn, step).await?;
        }
    }

    async fn claim_next_step(
        &self,
        turn: u64,
        step: u32,
    ) -> Result<Vec<super::session_log::HarnessInboxMessage>, HarnessDriverError> {
        let messages = match self.ports.inputs.drain().await {
            Ok(messages) => messages,
            Err(error) => {
                let error = HarnessDriverError::Port(error);
                self.close_turn_error(turn, &error)?;
                return Err(error);
            }
        };
        for (index, message) in messages.into_iter().enumerate() {
            if let Err(error) = self.inbox.send(super::agent::AgentSendRequest::inject(
                format!("external-{turn}-{step}-{index}"),
                message,
            )) {
                let error = HarnessDriverError::Inbox(error.to_string());
                self.close_turn_error(turn, &error)?;
                return Err(error);
            }
        }
        match self.inbox.claim_for_step() {
            Ok(claim) => Ok(claim.messages),
            Err(error) => {
                let error = HarnessDriverError::Inbox(error.to_string());
                self.close_turn_error(turn, &error)?;
                Err(error)
            }
        }
    }

    async fn execute_calls(
        &self,
        turn: u64,
        step: u32,
        calls: Vec<(HarnessToolInvocation, u64)>,
        cancellation: CancellationToken,
    ) -> Result<(), HarnessDriverError> {
        let mut cursor = 0;
        while cursor < calls.len() {
            let exclusive = self.ports.tools.execution_mode(&calls[cursor].0.name)
                == HarnessToolExecutionMode::Exclusive;
            let end = if exclusive {
                cursor + 1
            } else {
                let mut end = cursor + 1;
                while end < calls.len()
                    && self.ports.tools.execution_mode(&calls[end].0.name)
                        == HarnessToolExecutionMode::Parallel
                {
                    end += 1;
                }
                end
            };
            let futures = calls[cursor..end].iter().map(|(invocation, _)| {
                let invocation = invocation.clone();
                let tools = self.ports.tools;
                let hooks = self.ports.hooks;
                let token = cancellation.clone();
                async move {
                    if token.is_cancelled() {
                        HarnessToolOutput {
                            content: "cancelled before dispatch".into(),
                            is_error: true,
                            error_code: Some("ABORTED_BEFORE_DISPATCH".into()),
                        }
                    } else {
                        let context = HarnessStepContext { turn, step };
                        let mut output = match hooks.tool_pre(context, &invocation).await {
                            HarnessToolPreDecision::Execute => {
                                tools.execute(invocation.clone(), token).await
                            }
                            HarnessToolPreDecision::Return(output) => output,
                        };
                        hooks.tool_post(context, &invocation, &mut output).await;
                        output
                    }
                }
            });
            let outputs = join_all(futures).await;
            for ((invocation, call_seq), output) in calls[cursor..end].iter().zip(outputs) {
                self.append(
                    HarnessSessionEventKind::ToolResult {
                        turn,
                        step,
                        call_id: invocation.call_id.clone(),
                        content: output.content,
                        is_error: output.is_error,
                        error_code: output.error_code,
                    },
                    Some(SurfaceOp::Append),
                    vec![*call_seq],
                )?;
            }
            cursor = end;
        }
        Ok(())
    }

    fn append(
        &self,
        kind: HarnessSessionEventKind,
        surface: Option<SurfaceOp>,
        sources: Vec<u64>,
    ) -> Result<u64, HarnessDriverError> {
        Ok(self
            .session
            .lock()
            .map_err(|_| HarnessDriverError::Poisoned)?
            .append(self.ports.clock.wall_now().0, kind, surface, sources)?
            .seq)
    }

    fn close_step_and_turn(
        &self,
        turn: u64,
        step: u32,
        reason: TurnEndReason,
    ) -> Result<(), HarnessDriverError> {
        self.append(
            HarnessSessionEventKind::StepEnd { turn, step },
            None,
            vec![],
        )?;
        self.append(
            HarnessSessionEventKind::TurnEnd { turn, reason },
            None,
            vec![],
        )?;
        Ok(())
    }

    fn close_step_error(
        &self,
        turn: u64,
        step: u32,
        error: &HarnessDriverError,
    ) -> Result<(), HarnessDriverError> {
        self.close_step_and_turn(turn, step, Self::error_reason(error))
    }

    fn close_turn_error(
        &self,
        turn: u64,
        error: &HarnessDriverError,
    ) -> Result<(), HarnessDriverError> {
        self.append(
            HarnessSessionEventKind::TurnEnd {
                turn,
                reason: Self::error_reason(error),
            },
            None,
            vec![],
        )?;
        Ok(())
    }

    fn error_reason(error: &HarnessDriverError) -> TurnEndReason {
        let code = match error {
            HarnessDriverError::Session(_) => "session",
            HarnessDriverError::Inbox(_) => "inbox",
            HarnessDriverError::Port(error) => &error.code,
            HarnessDriverError::Poisoned => "poisoned",
            HarnessDriverError::InvalidAssistantMessage => "invalid_assistant_message",
            HarnessDriverError::ToolArguments(_) => "tool_arguments",
        };
        TurnEndReason::Error {
            code: code.to_owned(),
            message: error.to_string(),
        }
    }

    fn result(
        final_message: Option<Message>,
        inference_rounds: u64,
        provider_retries: u64,
        tool_calls: u64,
        active_context_tokens: Option<u64>,
        usages: &[InferenceUsage],
    ) -> HarnessTurnResult {
        HarnessTurnResult {
            final_message,
            inference_rounds,
            provider_retries,
            tool_calls,
            active_context_tokens,
            cumulative_usage: InferenceUsage::aggregate(usages),
        }
    }
}
