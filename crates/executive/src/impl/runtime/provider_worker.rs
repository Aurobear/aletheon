//! Provider-backed sub-agent runtime with bounded structured attempt results.

use crate::core::sub_agent::{SubAgentExecutionContext, SubAgentRuntime};
use crate::service::{CapabilityExecutionContext, CapabilityService};
use async_trait::async_trait;
use fabric::{
    AttemptEvidence, AttemptUsage, Clock, CognitiveRole, ContentBlock, FailureClass, LlmProvider,
    Message, PrincipalId, Role, RuntimeFailure, RuntimeId, RuntimeResult, SandboxRequirement,
    ToolDefinition,
};
use std::collections::HashSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// A generic provider runtime. Provider brands remain configuration concerns.
pub struct ProviderWorkerRuntime {
    runtime_id: RuntimeId,
    role: CognitiveRole,
    llm: Arc<dyn LlmProvider>,
    tools: Vec<ToolDefinition>,
    capability: Arc<dyn CapabilityService>,
    clock: Arc<dyn Clock>,
    max_steps: usize,
    max_persisted_bytes: usize,
    allowed_tools: HashSet<String>,
}

impl ProviderWorkerRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime_id: RuntimeId,
        role: CognitiveRole,
        llm: Arc<dyn LlmProvider>,
        tools: Vec<ToolDefinition>,
        capability: Arc<dyn CapabilityService>,
        clock: Arc<dyn Clock>,
        max_steps: usize,
        max_persisted_bytes: usize,
        allowed_tools: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            runtime_id,
            role,
            llm,
            tools,
            capability,
            clock,
            max_steps: max_steps.max(1),
            max_persisted_bytes,
            allowed_tools: allowed_tools.into_iter().collect(),
        }
    }

    pub fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    pub fn role(&self) -> CognitiveRole {
        self.role
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .filter(|tool| self.allowed_tools.contains(&tool.name))
            .cloned()
            .collect()
    }

    fn usage(&self, input_tokens: u64, output_tokens: u64, started_ms: u64) -> AttemptUsage {
        AttemptUsage {
            input_tokens,
            output_tokens,
            cost_usd: None,
            elapsed_ms: self.clock.mono_now().0.saturating_sub(started_ms),
        }
    }

    fn failure(
        &self,
        class: FailureClass,
        message: impl Into<String>,
        retryable: bool,
        usage: AttemptUsage,
        evidence: Vec<AttemptEvidence>,
    ) -> RuntimeFailure {
        RuntimeFailure {
            class,
            message: message.into(),
            retryable,
            usage,
            evidence,
        }
        .bounded_for_persistence(self.max_persisted_bytes)
    }

    async fn run_attempt_with_context(
        &self,
        task: &str,
        cancel: CancellationToken,
        execution: Option<SubAgentExecutionContext>,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.run_loop(task, cancel, execution).await
    }

    async fn run_loop(
        &self,
        task: &str,
        cancel: CancellationToken,
        execution: Option<SubAgentExecutionContext>,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.run_loop_inner(task, cancel, execution).await
    }
}

#[async_trait]
impl SubAgentRuntime for ProviderWorkerRuntime {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String> {
        self.run_attempt(task, cancel)
            .await
            .map(|result| result.output)
            .map_err(|failure| failure.message)
    }

    async fn run_attempt(
        &self,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.run_loop_inner(task, cancel, None).await
    }

    async fn run_in_context(
        &self,
        task: &str,
        cancel: CancellationToken,
        context: SubAgentExecutionContext,
    ) -> Result<String, String> {
        self.run_attempt_with_context(task, cancel, Some(context))
            .await
            .map(|result| result.output)
            .map_err(|failure| failure.message)
    }
}

impl ProviderWorkerRuntime {
    async fn run_loop_inner(
        &self,
        task: &str,
        cancel: CancellationToken,
        execution: Option<SubAgentExecutionContext>,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        let started_ms = self.clock.mono_now().0;
        let mut input_tokens = 0_u64;
        let mut output_tokens = 0_u64;
        let mut evidence = Vec::new();
        let mut messages = vec![Message::user(task)];
        let approval_authority = execution.as_ref().map(|context| {
            let working_dir = if context.working_dir.is_absolute() {
                context.working_dir.clone()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| "/tmp".into())
                    .join(&context.working_dir)
            };
            (
                fabric::ConnectionId::new(),
                fabric::ThreadId(context.session_id.clone()),
                fabric::TurnId::new(),
                fabric::WorkspacePolicy::from_resolved_roots(working_dir, vec![])
                    .expect("sub-agent working directory was resolved"),
            )
        });

        for _ in 0..self.max_steps {
            if cancel.is_cancelled() {
                return Err(self.failure(
                    FailureClass::Cancelled,
                    "sub-agent cancelled",
                    false,
                    self.usage(input_tokens, output_tokens, started_ms),
                    evidence,
                ));
            }

            let tool_defs = self.tool_definitions();
            let response = tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(self.failure(
                        FailureClass::Cancelled,
                        "sub-agent cancelled",
                        false,
                        self.usage(input_tokens, output_tokens, started_ms),
                        evidence,
                    ));
                }
                response = self.llm.complete(&messages, &tool_defs) => response,
            }
            .map_err(|error| {
                self.failure(
                    FailureClass::ProviderTransient,
                    format!("LLM error: {error}"),
                    true,
                    self.usage(input_tokens, output_tokens, started_ms),
                    evidence.clone(),
                )
            })?;
            input_tokens = input_tokens.saturating_add(response.usage.input_tokens.into());
            output_tokens = output_tokens.saturating_add(response.usage.output_tokens.into());

            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            if tool_calls.is_empty() {
                let output = if text_parts.is_empty() {
                    "(sub-agent produced no text output)".into()
                } else {
                    text_parts.join("\n")
                };
                return Ok(RuntimeResult {
                    output,
                    usage: self.usage(input_tokens, output_tokens, started_ms),
                    evidence,
                }
                .bounded_for_persistence(self.max_persisted_bytes));
            }

            messages.push(Message {
                role: Role::Assistant,
                content: response.content,
            });

            for (call_id, name, input) in tool_calls {
                if cancel.is_cancelled() {
                    return Err(self.failure(
                        FailureClass::Cancelled,
                        "sub-agent cancelled",
                        false,
                        self.usage(input_tokens, output_tokens, started_ms),
                        evidence,
                    ));
                }

                let (content, is_error) = if !self.allowed_tools.contains(&name) {
                    (format!("Tool not allowed: {name}"), true)
                } else {
                    let capability_context = execution.clone().zip(approval_authority.clone()).map(
                        |(context, (connection_id, thread_id, turn_id, workspace))| {
                            CapabilityExecutionContext {
                                agent: None,
                                process_id: context.process_id,
                                operation_id: context.operation_id,
                                principal: PrincipalId(format!("sub-agent:{}", self.runtime_id.0)),
                                connection_id,
                                thread_id,
                                turn_id,
                                workspace,
                                session_id: context.session_id,
                                working_dir: context.working_dir,
                                sandbox: SandboxRequirement::NotRequired,
                                cancel: cancel.clone(),
                                turn_count: 0,
                                action_loop: None,
                                streaming_tools: false,
                                turn_event_sender: None,
                            }
                        },
                    );
                    let result = self
                        .capability
                        .invoke(
                            capability_context,
                            fabric::CapabilityCall {
                                operation_id: execution
                                    .as_ref()
                                    .map(|context| context.operation_id)
                                    .unwrap_or_default(),
                                process_id: execution
                                    .as_ref()
                                    .map(|context| context.process_id)
                                    .unwrap_or_default(),
                                name: name.clone(),
                                input,
                                call_id: call_id.clone(),
                                deadline: None,
                            },
                            cancel.clone(),
                        )
                        .await;
                    (result.output, result.is_error)
                };
                evidence.push(AttemptEvidence {
                    kind: "tool_result".into(),
                    summary: format!("{}: {}", name, if is_error { "error" } else { "ok" }),
                    content: content.clone(),
                });
                messages.push(Message::tool_result(&call_id, &content, is_error));
            }
        }

        Err(self.failure(
            FailureClass::RepeatedFailure,
            "sub-agent exhausted its reasoning step limit",
            true,
            self.usage(input_tokens, output_tokens, started_ms),
            evidence,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use anyhow::anyhow;
    use corpus::tools::tools::ToolRegistry;
    use fabric::tool::{Tool, ToolContext, ToolResult, ToolResultMeta};
    use fabric::{LlmResponse, LlmStream, Registry, StopReason, Usage};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    struct TestCapabilityService(Arc<AtomicUsize>);

    #[async_trait]
    impl CapabilityService for TestCapabilityService {
        async fn invoke(
            &self,
            _context: Option<CapabilityExecutionContext>,
            call: fabric::CapabilityCall,
            _cancel: CancellationToken,
        ) -> fabric::CapabilityResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            fabric::CapabilityResult {
                call_id: call.call_id,
                output: "counted".into(),
                is_error: false,
                usage: fabric::UsageReport::default(),
                audit_id: Some(fabric::AuditEventId::new()),
            }
        }
    }

    fn test_capability(counter: Arc<AtomicUsize>) -> Arc<dyn CapabilityService> {
        Arc::new(TestCapabilityService(counter))
    }

    struct ScriptedProvider {
        responses: Mutex<VecDeque<anyhow::Result<LlmResponse>>>,
        seen_tools: Mutex<Vec<Vec<String>>>,
        advance_clock: Option<(Arc<TestClock>, u64)>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            self.seen_tools
                .lock()
                .await
                .push(tools.iter().map(|tool| tool.name.clone()).collect());
            if let Some((clock, millis)) = &self.advance_clock {
                clock.advance(*millis);
            }
            self.responses
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("script exhausted")))
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            Err(anyhow!("stream unused"))
        }

        fn name(&self) -> &str {
            "scripted"
        }

        fn max_context_length(&self) -> usize {
            4096
        }
    }

    #[derive(Clone)]
    struct CountingTool(Arc<AtomicUsize>);

    #[async_trait]
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "count"
        }

        fn description(&self) -> &str {
            "count calls"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn permission_level(&self) -> fabric::tool::PermissionLevel {
            fabric::tool::PermissionLevel::L0
        }

        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(self.clone())
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            ToolResult {
                content: "counted".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            }
        }
    }

    fn response(content: Vec<ContentBlock>, input: u32, output: u32) -> LlmResponse {
        LlmResponse {
            content,
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
            },
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        }
    }

    fn runtime(
        responses: Vec<anyhow::Result<LlmResponse>>,
        allowed: &[&str],
        max_steps: usize,
        counter: Arc<AtomicUsize>,
    ) -> (ProviderWorkerRuntime, Arc<ScriptedProvider>) {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(responses.into()),
            seen_tools: Mutex::new(Vec::new()),
            advance_clock: None,
        });
        let mut registry = ToolRegistry::new();
        registry
            .register(Arc::new(CountingTool(counter.clone())))
            .unwrap();
        let definitions = registry.definitions();
        let clock = Arc::new(TestClock::new(0, 0));
        (
            ProviderWorkerRuntime::new(
                RuntimeId("worker".into()),
                CognitiveRole::Worker,
                provider.clone(),
                definitions,
                test_capability(counter),
                clock,
                max_steps,
                1024,
                allowed.iter().map(|name| (*name).to_string()),
            ),
            provider,
        )
    }

    #[tokio::test]
    async fn end_turn_returns_bounded_text_and_usage() {
        let (runtime, _) = runtime(
            vec![Ok(response(
                vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                4,
                2,
            ))],
            &[],
            2,
            Arc::new(AtomicUsize::new(0)),
        );
        let result = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.output, "done");
        assert_eq!(result.usage.input_tokens, 4);
        assert_eq!(result.usage.output_tokens, 2);
    }

    #[tokio::test]
    async fn elapsed_time_comes_from_the_injected_clock() {
        let clock = Arc::new(TestClock::new(0, 100));
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(
                vec![Ok(response(
                    vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    1,
                    1,
                ))]
                .into(),
            ),
            seen_tools: Mutex::new(Vec::new()),
            advance_clock: Some((clock.clone(), 37)),
        });
        let runtime = ProviderWorkerRuntime::new(
            RuntimeId("worker".into()),
            CognitiveRole::Worker,
            provider,
            Vec::new(),
            test_capability(Arc::new(AtomicUsize::new(0))),
            clock,
            1,
            1024,
            Vec::<String>::new(),
        );

        let result = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.usage.elapsed_ms, 37);
    }

    #[tokio::test]
    async fn returned_output_is_bounded_before_persistence() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(
                vec![Ok(response(
                    vec![ContentBlock::Text {
                        text: "abcdefgh".into(),
                    }],
                    1,
                    1,
                ))]
                .into(),
            ),
            seen_tools: Mutex::new(Vec::new()),
            advance_clock: None,
        });
        let runtime = ProviderWorkerRuntime::new(
            RuntimeId("worker".into()),
            CognitiveRole::Worker,
            provider,
            Vec::new(),
            test_capability(Arc::new(AtomicUsize::new(0))),
            Arc::new(TestClock::default()),
            1,
            4,
            Vec::<String>::new(),
        );

        let result = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.output, "abcd");
    }

    #[tokio::test]
    async fn tool_loop_executes_allowed_tool_and_aggregates_usage() {
        let counter = Arc::new(AtomicUsize::new(0));
        let (runtime, provider) = runtime(
            vec![
                Ok(response(
                    vec![ContentBlock::ToolUse {
                        id: "call-1".into(),
                        name: "count".into(),
                        input: serde_json::json!({}),
                    }],
                    3,
                    1,
                )),
                Ok(response(
                    vec![ContentBlock::Text {
                        text: "finished".into(),
                    }],
                    5,
                    2,
                )),
            ],
            &["count"],
            3,
            counter.clone(),
        );
        let result = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(result.usage.input_tokens, 8);
        assert_eq!(result.usage.output_tokens, 3);
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(provider.seen_tools.lock().await[0], ["count"]);
    }

    #[tokio::test]
    async fn max_steps_is_a_structured_retryable_failure() {
        let (runtime, _) = runtime(
            vec![Ok(response(
                vec![ContentBlock::ToolUse {
                    id: "call-1".into(),
                    name: "count".into(),
                    input: serde_json::json!({}),
                }],
                1,
                1,
            ))],
            &["count"],
            1,
            Arc::new(AtomicUsize::new(0)),
        );
        let failure = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(failure.class, FailureClass::RepeatedFailure);
        assert!(failure.retryable);
    }

    #[tokio::test]
    async fn cancellation_has_a_distinct_non_retryable_class() {
        let (runtime, _) = runtime(vec![], &[], 1, Arc::new(AtomicUsize::new(0)));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let failure = runtime.run_attempt("task", cancel).await.unwrap_err();
        assert_eq!(failure.class, FailureClass::Cancelled);
        assert!(!failure.retryable);
    }

    #[tokio::test]
    async fn allow_list_hides_and_refuses_disallowed_tools() {
        let counter = Arc::new(AtomicUsize::new(0));
        let (runtime, provider) = runtime(
            vec![
                Ok(response(
                    vec![ContentBlock::ToolUse {
                        id: "call-1".into(),
                        name: "count".into(),
                        input: serde_json::json!({}),
                    }],
                    1,
                    1,
                )),
                Ok(response(
                    vec![ContentBlock::Text {
                        text: "handled".into(),
                    }],
                    1,
                    1,
                )),
            ],
            &[],
            2,
            counter.clone(),
        );
        let result = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.output, "handled");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(provider.seen_tools.lock().await[0].is_empty());
        assert!(result.evidence[0].content.contains("not allowed"));
    }

    #[tokio::test]
    async fn provider_error_is_structured_and_run_adapter_returns_message() {
        let (runtime, _) = runtime(
            vec![Err(anyhow!("temporary")), Err(anyhow!("temporary"))],
            &[],
            1,
            Arc::new(AtomicUsize::new(0)),
        );
        let failure = runtime
            .run_attempt("task", CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(failure.class, FailureClass::ProviderTransient);
        assert!(failure.retryable);
        assert!(runtime
            .run("task", CancellationToken::new())
            .await
            .unwrap_err()
            .contains("LLM error"));
    }
}
