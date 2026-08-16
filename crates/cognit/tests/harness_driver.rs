use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::harness::agent::{AgentInbox, AgentSendRequest};
use cognit::harness::driver::*;
use cognit::harness::lifecycle::*;
use cognit::harness::session_log::{
    HarnessRequestHeader, HarnessSessionEventKind, HarnessSessionId, HarnessSessionLog,
    TurnEndReason,
};
use contracts::{ContentBlock, InferenceUsage, Message, Role, StopReason, ToolDefinition};
use kernel::chronos::TestClock;
use tokio_util::sync::CancellationToken;

struct Prompt;
#[async_trait]
impl HarnessPromptAssembler for Prompt {
    async fn system_prompt(&self) -> Result<String, HarnessPortError> {
        Ok("system".into())
    }
}

struct Model(Mutex<VecDeque<HarnessModelResponse>>);
#[async_trait]
impl HarnessModelService for Model {
    async fn infer(
        &self,
        _: HarnessModelRequest,
        _: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError> {
        Ok(self.0.lock().unwrap().pop_front().unwrap())
    }
}

struct FailingModel;
#[async_trait]
impl HarnessModelService for FailingModel {
    async fn infer(
        &self,
        _: HarnessModelRequest,
        _: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError> {
        Err(HarnessPortError {
            code: "provider_unavailable".into(),
            message: "offline".into(),
        })
    }
}

struct ScriptedModel(Mutex<VecDeque<Result<HarnessModelResponse, HarnessPortError>>>);
#[async_trait]
impl HarnessModelService for ScriptedModel {
    async fn infer(
        &self,
        _: HarnessModelRequest,
        _: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError> {
        self.0.lock().unwrap().pop_front().unwrap()
    }
}

struct PolicyHooks;
#[async_trait]
impl HarnessLifecycleHooks for PolicyHooks {
    async fn prepare_request(
        &self,
        _: HarnessStepContext,
        system_prompt: &mut String,
        _: &mut Vec<ToolDefinition>,
    ) -> Result<(), HarnessPortError> {
        system_prompt.push_str(" + policy");
        Ok(())
    }
    async fn request_error(
        &self,
        _: HarnessStepContext,
        attempt: u32,
        _: &HarnessPortError,
    ) -> HarnessRequestErrorDecision {
        if attempt == 1 {
            HarnessRequestErrorDecision::Retry
        } else {
            HarnessRequestErrorDecision::Fail
        }
    }
    async fn tool_pre(
        &self,
        _: HarnessStepContext,
        _: &HarnessToolInvocation,
    ) -> HarnessToolPreDecision {
        HarnessToolPreDecision::Return(HarnessToolOutput {
            content: "policy denied".into(),
            is_error: true,
            error_code: Some("POLICY_DENIED".into()),
        })
    }
}

struct Tools;
#[async_trait]
impl HarnessToolService for Tools {
    async fn definitions(&self) -> Result<Vec<ToolDefinition>, HarnessPortError> {
        Ok(vec![ToolDefinition {
            name: "echo".into(),
            description: "echo input".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }])
    }
    fn execution_mode(&self, _: &str) -> HarnessToolExecutionMode {
        HarnessToolExecutionMode::Parallel
    }
    async fn execute(
        &self,
        invocation: HarnessToolInvocation,
        _: CancellationToken,
    ) -> HarnessToolOutput {
        HarnessToolOutput {
            content: invocation.input.to_string(),
            is_error: false,
            error_code: None,
        }
    }
}

struct BarrierTools {
    barrier: tokio::sync::Barrier,
    active: AtomicUsize,
    violated: AtomicBool,
}

#[async_trait]
impl HarnessToolService for BarrierTools {
    async fn definitions(&self) -> Result<Vec<ToolDefinition>, HarnessPortError> {
        Ok(["p1", "p2", "exclusive", "p3"]
            .into_iter()
            .map(|name| ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .collect())
    }
    fn execution_mode(&self, name: &str) -> HarnessToolExecutionMode {
        if name == "exclusive" {
            HarnessToolExecutionMode::Exclusive
        } else {
            HarnessToolExecutionMode::Parallel
        }
    }
    async fn execute(
        &self,
        invocation: HarnessToolInvocation,
        _: CancellationToken,
    ) -> HarnessToolOutput {
        if invocation.name == "p1" || invocation.name == "p2" {
            self.active.fetch_add(1, Ordering::SeqCst);
            self.barrier.wait().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
        } else if invocation.name == "exclusive" && self.active.load(Ordering::SeqCst) != 0 {
            self.violated.store(true, Ordering::SeqCst);
        }
        HarnessToolOutput {
            content: invocation.name,
            is_error: false,
            error_code: None,
        }
    }
}

fn response(message: Message, stop_reason: StopReason) -> HarnessModelResponse {
    HarnessModelResponse {
        chunks: vec![],
        message,
        usage: InferenceUsage::unsupported(Some(10), Some(2)),
        stop_reason,
        provider_retries: 1,
        active_context_tokens: Some(12),
    }
}

#[tokio::test]
async fn minimal_loop_records_exact_requests_and_tool_provenance() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("driver".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let tool_message = Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolUse {
            id: "call-1".into(),
            name: "echo".into(),
            input: serde_json::json!({"value": 1}),
        }],
    };
    let model = Arc::new(Model(Mutex::new(VecDeque::from([
        response(tool_message, StopReason::ToolUse),
        response(Message::assistant("done"), StopReason::EndTurn),
    ]))));
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: model.as_ref(),
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );
    let result = driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.inference_rounds, 2);
    assert_eq!(result.provider_retries, 2);
    assert_eq!(result.tool_calls, 1);
    assert_eq!(result.active_context_tokens, Some(12));
    assert_eq!(result.cumulative_usage.total_input_tokens, Some(20));
    let log = session.lock().unwrap();
    assert_eq!(log.derive_messages().len(), 4);
    assert_eq!(
        log.events()
            .iter()
            .filter(|event| matches!(event.kind, HarnessSessionEventKind::RequestPrepared { .. }))
            .count(),
        2
    );
    assert!(matches!(
        &log.events().last().unwrap().kind,
        HarnessSessionEventKind::TurnEnd {
            reason: TurnEndReason::Completed,
            ..
        }
    ));
}

#[tokio::test]
async fn cancellation_before_model_dispatch_closes_the_turn() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("cancel".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let empty_model = Model(Mutex::new(VecDeque::new()));
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &empty_model,
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );
    let token = CancellationToken::new();
    token.cancel();
    let result = driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            token,
        )
        .await
        .unwrap();
    assert_eq!(result.inference_rounds, 0);
    assert!(matches!(
        &session.lock().unwrap().events().last().unwrap().kind,
        HarnessSessionEventKind::TurnEnd {
            reason: TurnEndReason::Aborted { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn empty_turn_is_blocked_and_provider_error_is_durably_closed() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("errors".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    let empty_driver = HarnessLoopDriver::new(
        session.clone(),
        inbox.clone(),
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &FailingModel,
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock: clock.clone(),
        },
    );
    let header = HarnessRequestHeader {
        provider: "test".into(),
        model: "model".into(),
        reasoning_effort: None,
        max_output_tokens: None,
    };
    assert!(empty_driver
        .run_turn(header.clone(), CancellationToken::new())
        .await
        .unwrap()
        .final_message
        .is_none());
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let error = empty_driver
        .run_turn(header, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, HarnessDriverError::Port(_)));

    let events = session.lock().unwrap().events().to_vec();
    assert!(matches!(
        events[events.len() - 1].kind,
        HarnessSessionEventKind::TurnEnd {
            reason: TurnEndReason::Error { .. },
            ..
        }
    ));
    let restored = HarnessSessionLog::restore(HarnessSessionId("errors".into()), events).unwrap();
    assert_eq!(restored.next_turn_number(), 3);
    // The next turn resets its step counter when TurnStart is appended.
    assert_eq!(restored.next_step_number(), 1);
    let restored = Arc::new(Mutex::new(restored));
    let restored_inbox = Arc::new(AgentInbox::new(restored.clone(), clock.clone()));
    restored_inbox
        .send(AgentSendRequest::followup("two", Message::user("resume")))
        .unwrap();
    let resumed_model = Model(Mutex::new(VecDeque::from([response(
        Message::assistant("resumed"),
        StopReason::EndTurn,
    )])));
    let resumed = HarnessLoopDriver::new(
        restored.clone(),
        restored_inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &resumed_model,
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );
    resumed
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(restored.lock().unwrap().next_turn_number(), 4);
}

#[tokio::test]
async fn parallel_batches_preserve_exclusive_barriers_and_result_order() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("barrier".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let calls = ["p1", "p2", "exclusive", "p3"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| ContentBlock::ToolUse {
            id: format!("call-{index}"),
            name: name.into(),
            input: serde_json::json!({}),
        })
        .collect();
    let model = Arc::new(Model(Mutex::new(VecDeque::from([
        response(
            Message {
                role: Role::Assistant,
                content: calls,
            },
            StopReason::ToolUse,
        ),
        response(Message::assistant("done"), StopReason::EndTurn),
    ]))));
    let tools = Arc::new(BarrierTools {
        barrier: tokio::sync::Barrier::new(2),
        active: AtomicUsize::new(0),
        violated: AtomicBool::new(false),
    });
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: model.as_ref(),
            inputs: &NoopHarnessStepInputSource,
            tools: tools.as_ref(),
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );
    driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!tools.violated.load(Ordering::SeqCst));
    let result_ids = session
        .lock()
        .unwrap()
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            HarnessSessionEventKind::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(result_ids, vec!["call-0", "call-1", "call-2", "call-3"]);
}

#[tokio::test]
async fn typed_hooks_prepare_retry_and_intercept_without_entering_loop_policy() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("hooks".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let tool_message = Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolUse {
            id: "blocked".into(),
            name: "echo".into(),
            input: serde_json::json!({}),
        }],
    };
    let model = Arc::new(ScriptedModel(Mutex::new(VecDeque::from([
        Err(HarnessPortError {
            code: "backpressure".into(),
            message: "retry".into(),
        }),
        Ok(response(tool_message, StopReason::ToolUse)),
        Ok(response(Message::assistant("done"), StopReason::EndTurn)),
    ]))));
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: model.as_ref(),
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &PolicyHooks,
            clock,
        },
    );
    driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let events = session.lock().unwrap().events().to_vec();
    assert!(events.iter().any(|event| matches!(
        event.kind,
        HarnessSessionEventKind::RequestError {
            will_retry: true,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        HarnessSessionEventKind::RequestPrepared { system_prompt, .. }
            if system_prompt == "system + policy"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        HarnessSessionEventKind::ToolResult { error_code: Some(code), .. }
            if code == "POLICY_DENIED"
    )));
}

struct QueuedInputs(Mutex<VecDeque<Vec<Message>>>);

#[async_trait]
impl HarnessStepInputSource for QueuedInputs {
    async fn drain(&self) -> Result<Vec<Message>, HarnessPortError> {
        Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
    }
}

struct RequestRecordingModel {
    responses: Mutex<VecDeque<HarnessModelResponse>>,
    requests: Mutex<Vec<HarnessModelRequest>>,
}

#[async_trait]
impl HarnessModelService for RequestRecordingModel {
    async fn infer(
        &self,
        request: HarnessModelRequest,
        _: CancellationToken,
    ) -> Result<HarnessModelResponse, HarnessPortError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.responses.lock().unwrap().pop_front().unwrap())
    }
}

#[tokio::test]
async fn step_input_is_claimed_before_a_completed_turn_closes() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("step-input".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let model = RequestRecordingModel {
        responses: Mutex::new(VecDeque::from([
            response(Message::assistant("draft"), StopReason::EndTurn),
            response(Message::assistant("final"), StopReason::EndTurn),
        ])),
        requests: Mutex::new(Vec::new()),
    };
    let inputs = QueuedInputs(Mutex::new(VecDeque::from([
        vec![Message::user("steer before closing")],
        vec![],
    ])));
    let driver = HarnessLoopDriver::new(
        session,
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &model,
            inputs: &inputs,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );

    let result = driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.inference_rounds, 2);
    let requests = model.requests.lock().unwrap();
    assert!(requests[1].messages.iter().any(|message| {
        message.content.iter().any(
            |block| matches!(block, ContentBlock::Text { text } if text == "steer before closing"),
        )
    }));
}

struct ReversePlanningTools(Mutex<Vec<String>>);

#[async_trait]
impl HarnessToolService for ReversePlanningTools {
    async fn definitions(&self) -> Result<Vec<ToolDefinition>, HarnessPortError> {
        Ok(Vec::new())
    }

    async fn plan(
        &self,
        mut invocations: Vec<HarnessToolInvocation>,
    ) -> Result<Vec<HarnessToolInvocation>, HarnessPortError> {
        invocations.reverse();
        Ok(invocations)
    }

    fn execution_mode(&self, _: &str) -> HarnessToolExecutionMode {
        HarnessToolExecutionMode::Exclusive
    }

    async fn execute(
        &self,
        invocation: HarnessToolInvocation,
        _: CancellationToken,
    ) -> HarnessToolOutput {
        self.0.lock().unwrap().push(invocation.name.clone());
        HarnessToolOutput {
            content: invocation.name,
            is_error: false,
            error_code: None,
        }
    }
}

#[tokio::test]
async fn tool_plan_is_applied_before_dispatch_and_durable_result_order() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("planned-tools".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let model = Model(Mutex::new(VecDeque::from([
        response(
            Message {
                role: Role::Assistant,
                content: ["first", "second"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| ContentBlock::ToolUse {
                        id: format!("call-{index}"),
                        name: name.into(),
                        input: serde_json::json!({}),
                    })
                    .collect(),
            },
            StopReason::ToolUse,
        ),
        response(Message::assistant("done"), StopReason::EndTurn),
    ])));
    let tools = ReversePlanningTools(Mutex::new(Vec::new()));
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &model,
            inputs: &NoopHarnessStepInputSource,
            tools: &tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );

    driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(tools.0.lock().unwrap().as_slice(), ["second", "first"]);
    let durable_results = session
        .lock()
        .unwrap()
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            HarnessSessionEventKind::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(durable_results, ["second", "first"]);
}

#[tokio::test]
async fn max_tokens_is_terminal_but_not_normal_completion() {
    let clock = Arc::new(TestClock::new(100, 0));
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("max-tokens".into())).unwrap(),
    ));
    let inbox = Arc::new(AgentInbox::new(session.clone(), clock.clone()));
    inbox
        .send(AgentSendRequest::followup("one", Message::user("go")))
        .unwrap();
    let model = Model(Mutex::new(VecDeque::from([response(
        Message::assistant("partial"),
        StopReason::MaxTokens,
    )])));
    let driver = HarnessLoopDriver::new(
        session.clone(),
        inbox,
        HarnessLoopPorts {
            prompt: &Prompt,
            model: &model,
            inputs: &NoopHarnessStepInputSource,
            tools: &Tools,
            hooks: &NoopHarnessLifecycleHooks,
            clock,
        },
    );

    driver
        .run_turn(
            HarnessRequestHeader {
                provider: "test".into(),
                model: "model".into(),
                reasoning_effort: None,
                max_output_tokens: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(matches!(
        session.lock().unwrap().events().last().unwrap().kind,
        HarnessSessionEventKind::TurnEnd {
            reason: TurnEndReason::MaxTokens,
            ..
        }
    ));
}

#[test]
fn minimal_loop_source_does_not_reabsorb_legacy_policy_state() {
    let source = include_str!("../src/harness/driver.rs");
    for forbidden in [
        "CompactorTrait",
        "GoalTracker",
        "AwarenessEngine",
        "CompletionAuditor",
        "RepositoryInspection",
        "ReflectionEngine",
    ] {
        assert!(
            !source.contains(forbidden),
            "minimal driver reintroduced legacy policy dependency: {forbidden}"
        );
    }
}
