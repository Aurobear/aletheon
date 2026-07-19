use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use cognit::{CanonicalTurnEventSink, CognitiveStreamEvent};
use fabric::{
    CapabilityCall, CapabilityResult, DaseinView, LlmProvider, Message, RecallRequest, RecallSet,
    SessionId, SessionRecord, SessionStatus, ToolDefinition, TurnRequest, TurnResult, TurnServices,
    SESSION_SCHEMA_VERSION,
};
use tokio_util::sync::CancellationToken;

use crate::core::config::ExecutiveConfig;
use crate::service::harness_factory::CognitiveSessionFactory;
use crate::service::turn_policy::TurnPolicy;

pub struct DaemonStreamingTurnContext<F> {
    pub config: ExecutiveConfig,
    pub llm: Arc<dyn LlmProvider>,
    pub tool_defs: Vec<ToolDefinition>,
    pub execute_tool: F,
    pub event_sink: CanonicalTurnEventSink,
    pub request_messages: Vec<Message>,
    pub dasein_context: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    pub cancel_token: CancellationToken,
    pub sessions: Arc<dyn CognitiveSessionFactory>,
    pub batch_planner: Option<Arc<dyn cognit::harness::BatchPlanner>>,
    pub session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
    pub prompt_queue_enabled: bool,
}

/// Submit one daemon turn through Cognit's authoritative session facade.
pub async fn submit_streaming_daemon_turn<F, Fut>(
    request: TurnRequest,
    context: DaemonStreamingTurnContext<F>,
) -> anyhow::Result<TurnResult>
where
    F: Fn(&str, &str, &serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = (String, bool)> + Send + 'static,
{
    let DaemonStreamingTurnContext {
        config,
        llm,
        tool_defs,
        execute_tool,
        event_sink,
        request_messages,
        dasein_context,
        cancel_token,
        sessions,
        batch_planner,
        session_input,
        prompt_queue_enabled,
    } = context;
    let services = DaemonTurnServices {
        llm,
        tool_defs,
        execute_tool,
        request_messages,
        dasein_context,
        session_input,
        prompt_queue_enabled,
        principal_id: request.context.principal_id.clone(),
        thread_id: request.context.thread_id.clone(),
        receipt_prefix: request.operation_id.0.to_string(),
    };
    let session_record = SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: SessionId(request.context.thread_id.0.clone()),
        parent: None,
        created_at_ms: 0,
        status: SessionStatus::Active,
    };
    let harness_config = crate::service::harness_factory::harness_config_from_executive(&config);
    let mut session = sessions
        .create_configured_with_batch_planner(
            &session_record,
            &TurnPolicy::daemon(),
            harness_config,
            cancel_token,
            batch_planner,
        )
        .await?;
    Ok(session
        .run_streaming_turn(request, &services, &fabric::NoopTurnEventSink, &event_sink)
        .await?)
}

struct DaemonTurnServices<F> {
    llm: Arc<dyn LlmProvider>,
    tool_defs: Vec<ToolDefinition>,
    execute_tool: F,
    request_messages: Vec<Message>,
    dasein_context: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
    prompt_queue_enabled: bool,
    principal_id: fabric::PrincipalId,
    thread_id: fabric::ThreadId,
    receipt_prefix: String,
}

#[async_trait]
impl<F, Fut> TurnServices for DaemonTurnServices<F>
where
    F: Fn(&str, &str, &serde_json::Value) -> Fut + Send + Sync,
    Fut: Future<Output = (String, bool)> + Send,
{
    async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: fabric::ProcessId) -> anyhow::Result<DaseinView> {
        Ok(DaseinView {
            text: (self.dasein_context)(),
        })
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        let (output, is_error) = (self.execute_tool)(&call.call_id, &call.name, &call.input).await;
        CapabilityResult {
            call_id: call.call_id,
            output,
            is_error,
            usage: fabric::UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        }
    }

    async fn drain_interjections(&self) -> anyhow::Result<Vec<String>> {
        if !self.prompt_queue_enabled {
            return Ok(Vec::new());
        }
        self.session_input
            .drain_interjections_at_safe_point(
                &self.principal_id,
                &self.thread_id,
                &self.receipt_prefix,
            )
            .await
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(self.llm.as_ref())
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_defs.clone()
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        self.request_messages.clone()
    }
}

pub type DaemonCognitiveEvent = CognitiveStreamEvent;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use corpus::tools::tools::structured_patch::{FileChangeSummary, StructuredPatchResult};
    use fabric::{
        ConnectionId, ContentBlock, LlmResponse, LlmStream, PrincipalId, PromptKind, Role,
        StopReason, ThreadId, Usage,
    };
    use std::sync::Mutex;

    struct RecordingLlm(Mutex<Vec<Vec<Message>>>);

    #[async_trait]
    impl LlmProvider for RecordingLlm {
        async fn complete(
            &self,
            messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            self.0.lock().unwrap().push(messages.to_vec());
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unreachable!()
        }

        fn name(&self) -> &str {
            "recording"
        }

        fn max_context_length(&self) -> usize {
            100_000
        }
    }

    #[tokio::test]
    async fn next_model_call_receives_turn_diff_after_first_tool_batch() {
        let principal = PrincipalId("p3-principal".into());
        let thread = ThreadId("p3-thread".into());
        let session_input =
            Arc::new(crate::service::session_input::SessionInputCoordinator::in_memory());
        let mut tracker = crate::service::turn_diff_tracker::TurnDiffTracker::default();
        tracker.record_patch(&StructuredPatchResult {
            applied: vec![],
            failed: vec![],
            files_changed: vec![FileChangeSummary {
                path: "src/main.rs".into(),
                change_type: "modified".into(),
                hunks_applied: 1,
                bytes_before: 10,
                bytes_after: 20,
            }],
        });
        session_input
            .enqueue(
                principal.clone(),
                ConnectionId::new(),
                thread.clone(),
                PromptKind::Interjection,
                tracker.to_context_injection(),
                "turn-diff:first-tool-batch".into(),
            )
            .await
            .unwrap();

        let llm = Arc::new(RecordingLlm(Mutex::new(Vec::new())));
        let services = DaemonTurnServices {
            llm: llm.clone(),
            tool_defs: vec![],
            execute_tool: |_id: &str, _name: &str, _input: &serde_json::Value| async {
                (String::new(), false)
            },
            request_messages: vec![Message::user("initial request")],
            dasein_context: Arc::new(|| None),
            session_input,
            prompt_queue_enabled: true,
            principal_id: principal,
            thread_id: thread,
            receipt_prefix: "p3-turn".into(),
        };

        let mut next_call_messages = services.request_messages.clone();
        next_call_messages.extend(
            services
                .drain_interjections()
                .await
                .unwrap()
                .into_iter()
                .map(Message::user),
        );
        services
            .llm_provider()
            .unwrap()
            .complete(&next_call_messages, &[])
            .await
            .unwrap();

        let received = llm.0.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].iter().any(|message| {
            message.role == Role::User
                && message.content.iter().any(|block| {
                    matches!(block, ContentBlock::Text { text } if text.contains("## Files changed this turn"))
                })
        }));
    }
}
