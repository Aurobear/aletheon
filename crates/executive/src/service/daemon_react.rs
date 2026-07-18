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
