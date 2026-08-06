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

use crate::application::harness_factory::CognitiveSessionFactory;
use crate::application::prefix_cache_observability::{record_prefix_shape_miss, LocalMissReason};
use crate::application::turn_policy::TurnPolicy;
use crate::composition::config::ExecutiveConfig;

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
    pub session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
    pub prompt_queue_enabled: bool,
    pub capability_receipts: Arc<tokio::sync::Mutex<Vec<fabric::CapabilityTerminalReceipt>>>,
    pub inference_items: Arc<tokio::sync::Mutex<Vec<fabric::ItemPayload>>>,
    pub prefix_shape_digest: Option<String>,
    pub local_cache_miss_reason: Option<LocalMissReason>,
    pub provider_miss_inference_allowed: bool,
}

/// Submit one daemon turn through Cognit's authoritative session facade.
pub async fn submit_streaming_daemon_turn<F, Fut, O>(
    request: TurnRequest,
    context: DaemonStreamingTurnContext<F>,
) -> anyhow::Result<TurnResult>
where
    F: Fn(&str, &str, &serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = O> + Send + 'static,
    O: Into<cognit::harness::event_sink::ToolResultEvent> + Send + 'static,
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
        capability_receipts,
        inference_items,
        prefix_shape_digest,
        local_cache_miss_reason,
        provider_miss_inference_allowed,
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
        capability_receipts,
        inference_items,
        prefix_shape_digest,
        local_cache_miss_reason: tokio::sync::Mutex::new(local_cache_miss_reason),
        provider_miss_inference_allowed,
    };
    let session_record = SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: SessionId(request.context.thread_id.0.clone()),
        parent: None,
        created_at_ms: 0,
        status: SessionStatus::Active,
    };
    let harness_config =
        crate::application::harness_factory::harness_config_from_executive(&config);
    let mut session = sessions
        .create_configured_with_batch_planner(
            &session_record,
            &TurnPolicy::daemon(),
            harness_config,
            cancel_token,
            batch_planner,
        )
        .await?;
    let runtime_events = event_sink.runtime_sink();
    Ok(session
        .run_streaming_turn(request, &services, &runtime_events, &event_sink)
        .await?)
}

struct DaemonTurnServices<F> {
    llm: Arc<dyn LlmProvider>,
    tool_defs: Vec<ToolDefinition>,
    execute_tool: F,
    request_messages: Vec<Message>,
    dasein_context: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
    prompt_queue_enabled: bool,
    principal_id: fabric::PrincipalId,
    thread_id: fabric::ThreadId,
    receipt_prefix: String,
    capability_receipts: Arc<tokio::sync::Mutex<Vec<fabric::CapabilityTerminalReceipt>>>,
    inference_items: Arc<tokio::sync::Mutex<Vec<fabric::ItemPayload>>>,
    prefix_shape_digest: Option<String>,
    local_cache_miss_reason: tokio::sync::Mutex<Option<LocalMissReason>>,
    provider_miss_inference_allowed: bool,
}

#[async_trait]
impl<F, Fut, O> TurnServices for DaemonTurnServices<F>
where
    F: Fn(&str, &str, &serde_json::Value) -> Fut + Send + Sync,
    Fut: Future<Output = O> + Send,
    O: Into<cognit::harness::event_sink::ToolResultEvent> + Send,
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
        let result: cognit::harness::event_sink::ToolResultEvent =
            (self.execute_tool)(&call.call_id, &call.name, &call.input)
                .await
                .into();
        CapabilityResult {
            call_id: call.call_id,
            output: result.content,
            is_error: result.is_error,
            usage: fabric::UsageReport {
                wall_time_ms: result.execution_time_ms,
                ..Default::default()
            },
            audit_id: None,
            patch_delta: result.patch_delta,
            served_from_cache: false,
        }
    }

    async fn record_capability_receipt(&self, receipt: fabric::CapabilityTerminalReceipt) {
        self.capability_receipts.lock().await.push(receipt);
    }

    async fn record_model_context_projection(
        &self,
        mut receipt: fabric::model_projection::ModelContextProjectionReceipt,
    ) {
        crate::composition::turn_service::materialize_projection_artifacts(&mut receipt);
        self.inference_items
            .lock()
            .await
            .push(fabric::ItemPayload::ModelContextProjection { receipt });
    }

    async fn record_inference_receipt(
        &self,
        mut receipt: fabric::types::inference_receipt::InferenceTerminalReceipt,
    ) {
        receipt.prefix_shape_digest = self.prefix_shape_digest.clone();
        let mut reason = self.local_cache_miss_reason.lock().await.take();
        if reason.is_none()
            && self.provider_miss_inference_allowed
            && receipt.usage.cache_telemetry == fabric::CacheTelemetry::Reported
            && receipt.usage.cache_read_tokens == Some(0)
            && receipt.usage.uncached_input_tokens.unwrap_or(0) > 0
        {
            reason = Some(LocalMissReason::ProviderMissOrEviction);
        }
        if let Some(reason) = reason {
            receipt.local_cache_miss_reason = Some(reason.as_str().to_owned());
            record_prefix_shape_miss(reason);
        }
        self.inference_items
            .lock()
            .await
            .push(fabric::ItemPayload::InferenceReceipt { receipt });
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
        ConnectionId, ContentBlock, InferenceUsage, LlmResponse, LlmStream, PrincipalId,
        PromptKind, Role, StopReason, ThreadId,
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
                usage: InferenceUsage::default(),
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
            Arc::new(crate::application::session_input::SessionInputCoordinator::in_memory());
        let mut tracker = crate::application::turn_diff_tracker::TurnDiffTracker::default();
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
            capability_receipts: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            inference_items: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            prefix_shape_digest: None,
            local_cache_miss_reason: tokio::sync::Mutex::new(None),
            provider_miss_inference_allowed: false,
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

    #[tokio::test]
    async fn daemon_services_publish_terminal_receipts_to_turn_artifacts() {
        let receipts = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let inference_items = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let services = DaemonTurnServices {
            llm: Arc::new(RecordingLlm(Mutex::new(Vec::new()))),
            tool_defs: vec![],
            execute_tool: |_id: &str, _name: &str, _input: &serde_json::Value| async {
                (String::new(), false)
            },
            request_messages: vec![],
            dasein_context: Arc::new(|| None),
            session_input: Arc::new(
                crate::application::session_input::SessionInputCoordinator::in_memory(),
            ),
            prompt_queue_enabled: false,
            principal_id: PrincipalId("test".into()),
            thread_id: ThreadId("test".into()),
            receipt_prefix: "test".into(),
            capability_receipts: receipts.clone(),
            inference_items: inference_items.clone(),
            prefix_shape_digest: Some("sha256:shape".into()),
            local_cache_miss_reason: tokio::sync::Mutex::new(Some(LocalMissReason::SystemChanged)),
            provider_miss_inference_allowed: false,
        };
        let receipt = fabric::CapabilityTerminalReceipt {
            invocation_id: "validation-1".into(),
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            capability: "validation_run".into(),
            status: fabric::CapabilityTerminalStatus::Succeeded,
            started_at: fabric::MonoTime(1),
            finished_at: fabric::MonoTime(2),
            exit_code: Some(0),
            error_class: None,
            artifact_ids: vec![],
            evidence_ids: vec![],
            output_ref: None,
            truncated: false,
            retry_disposition: fabric::CapabilityRetryDisposition::Never,
            audit_id: None,
        };
        services.record_capability_receipt(receipt).await;
        let retained = receipts.lock().await;
        assert_eq!(retained.len(), 1);
        assert!(retained[0].proves_success());
        drop(retained);

        services
            .record_inference_receipt(fabric::types::inference_receipt::InferenceTerminalReceipt {
                schema_version:
                    fabric::types::inference_receipt::INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
                inference_id: "inference-1".into(),
                operation_id: "operation-1".into(),
                provider_id: "provider".into(),
                model_id: "model".into(),
                system_prefix_digest: "sha256:system".into(),
                tool_schema_digest: "sha256:tools".into(),
                status: fabric::types::inference_receipt::InferenceTerminalStatus::Succeeded,
                usage: fabric::InferenceUsage::reported(10, 2, Some(10), Some(0), None),
                failure_kind: None,
                prefix_shape_digest: None,
                local_cache_miss_reason: None,
            })
            .await;
        let inference_items = inference_items.lock().await;
        let fabric::ItemPayload::InferenceReceipt { receipt } = &inference_items[0] else {
            panic!("expected inference receipt");
        };
        assert_eq!(receipt.prefix_shape_digest.as_deref(), Some("sha256:shape"));
        assert_eq!(
            receipt.local_cache_miss_reason.as_deref(),
            Some("system_changed")
        );
    }
}
