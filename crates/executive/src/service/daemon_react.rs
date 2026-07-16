use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use cognit::{ChannelCognitiveStreamSink, CognitiveStreamEvent};
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
    pub event_sink: ChannelCognitiveStreamSink,
    pub request_messages: Vec<Message>,
    pub dasein_context: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    pub cancel_token: CancellationToken,
    pub sessions: Arc<dyn CognitiveSessionFactory>,
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
    } = context;
    let services = DaemonTurnServices {
        llm,
        tool_defs,
        execute_tool,
        request_messages,
        dasein_context,
    };
    let session_record = SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: SessionId(request.session_id.clone()),
        parent: None,
        created_at_ms: 0,
        status: SessionStatus::Active,
    };
    let harness_config = crate::service::harness_factory::harness_config_from_executive(&config);
    let mut session = sessions
        .create_configured(
            &session_record,
            &TurnPolicy::daemon(),
            harness_config,
            cancel_token,
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
