use crate::adapters::session::canonical_store::CanonicalSessionStore;
use crate::application::harness_factory::{CognitiveSessionFactory, LinearCognitiveSessionFactory};
use crate::application::turn_coordinator::{cancelled_result, TurnCoordinator, TurnExecution};
use crate::application::turn_policy::TurnPolicy;
use crate::application::{PostTurnPipeline, PreTurnPipeline};
use anyhow::Result;
use async_trait::async_trait;
use cognit::harness::HarnessConfig;
use fabric::{
    CapabilityCall, CapabilityResult, Clock, ItemPayload, RecallRequest, RecallSet, Timer,
    TurnEventSink, TurnRequest, TurnServices,
};
use kernel::chronos::SystemTimer;
use kernel::KernelRuntime;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Compatibility facade over the canonical [`TurnCoordinator`].
pub struct TurnService {
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
    factory: Arc<dyn CognitiveSessionFactory>,
    clock: Arc<dyn Clock>,
    coordinator: Arc<TurnCoordinator>,
    policy: TurnPolicy,
}

impl TurnService {
    pub fn new(
        services: Arc<dyn TurnServices>,
        pre_turn: PreTurnPipeline,
        post_turn: PostTurnPipeline,
        kernel: Arc<KernelRuntime>,
    ) -> Self {
        let store =
            Arc::new(CanonicalSessionStore::open(":memory:").expect("in-memory session store"));
        let coordinator = Arc::new(
            crate::composition::turn_coordinator::compose_in_memory_turn_coordinator(
                kernel.clone(),
                store,
            ),
        );
        Self {
            services,
            pre_turn,
            post_turn,
            factory: Arc::new(LinearCognitiveSessionFactory::new(
                HarnessConfig::default(),
                kernel.clock(),
            )),
            clock: kernel.clock(),
            coordinator,
            policy: TurnPolicy::exec(),
        }
    }

    pub fn with_harness_config(mut self, harness_config: HarnessConfig) -> Self {
        self.factory = Arc::new(LinearCognitiveSessionFactory::new(
            harness_config,
            self.clock.clone(),
        ));
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_coordinator(mut self, coordinator: Arc<TurnCoordinator>) -> Self {
        self.coordinator = coordinator;
        self
    }

    pub fn with_session_factory(mut self, factory: Arc<dyn CognitiveSessionFactory>) -> Self {
        self.factory = factory;
        self
    }

    pub fn with_policy(mut self, policy: TurnPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub async fn submit(
        &self,
        request: TurnRequest,
        events: &dyn TurnEventSink,
    ) -> Result<fabric::TurnResult> {
        let services = self.services.clone();
        let pre_turn = self.pre_turn.clone();
        let factory = self.factory.clone();
        let clock = self.clock.clone();
        let policy = self.policy.clone();
        let runner_policy = policy.clone();
        let history_store = self.coordinator.store();
        let result = self
            .coordinator
            .submit_with(request, &policy, move |request, cancel| async move {
                let session_record = fabric::SessionRecord {
                    schema_version: fabric::SESSION_SCHEMA_VERSION,
                    id: fabric::SessionId(request.context.thread_id.0.clone()),
                    parent: None,
                    created_at_ms: 0,
                    status: fabric::SessionStatus::Active,
                };
                let mut history = history_store
                    .load_items(&fabric::SessionId(request.context.thread_id.0.clone()), None)
                    .await?;
                if history.last().is_some_and(|item| {
                    matches!(&item.payload, ItemPayload::UserMessage { content } if content == &request.input)
                }) {
                    history.pop();
                }
                let canonical_seed = crate::application::session_projection::project_messages(&history)?;
                let recording = RecordingTurnServices::new(services, canonical_seed);
                let request = pre_turn.run(request, &recording).await?;
                let mut session = factory
                    .create(&session_record, &runner_policy, cancel.clone())
                    .await?;
                let start = clock.mono_now();
                let run = session.run_turn(request.clone(), &recording, events);
                let mut result = match request.deadline {
                    Some(deadline) => tokio::select! {
                        _ = cancel.cancelled() => cancelled_result(),
                        timeout = SystemTimer.timeout(Duration::from_millis(deadline.0), run) => {
                            match timeout { Ok(result) => result?, Err(_) => cancelled_result() }
                        }
                    },
                    None => tokio::select! {
                        _ = cancel.cancelled() => cancelled_result(),
                        result = run => result?,
                    },
                };
                result.metrics.elapsed_ms = clock.mono_now().0.saturating_sub(start.0);
                Ok(TurnExecution {
                    result,
                    items: recording.take_items().await,
                    projection: None,
                    context_projection: None,
                })
            })
            .await?;
        self.post_turn.run(result).await
    }
}

struct RecordingTurnServices {
    inner: Arc<dyn TurnServices>,
    items: Mutex<Vec<ItemPayload>>,
    canonical_seed: Vec<fabric::Message>,
}

impl RecordingTurnServices {
    fn new(inner: Arc<dyn TurnServices>, canonical_seed: Vec<fabric::Message>) -> Self {
        Self {
            inner,
            items: Mutex::new(Vec::new()),
            canonical_seed,
        }
    }
    async fn take_items(&self) -> Vec<ItemPayload> {
        std::mem::take(&mut *self.items.lock().await)
    }
}

#[async_trait]
impl TurnServices for RecordingTurnServices {
    async fn recall(&self, request: RecallRequest) -> Result<RecallSet> {
        self.inner.recall(request).await
    }
    async fn dasein_view(&self, process: fabric::ProcessId) -> Result<fabric::DaseinView> {
        self.inner.dasein_view(process).await
    }
    async fn agora_view(&self, session_id: &str) -> Result<fabric::AgoraView> {
        self.inner.agora_view(session_id).await
    }
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        self.items.lock().await.push(ItemPayload::ToolCall {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
        });
        let result = self.inner.invoke(call).await;
        self.items.lock().await.push(ItemPayload::ToolResult {
            call_id: result.call_id.clone(),
            content: result.output.clone(),
            is_error: result.is_error,
            permit_id: (result.usage.permit_id != fabric::PermitId::default())
                .then_some(result.usage.permit_id),
            audit_id: result.audit_id,
        });
        result
    }
    async fn record_capability_receipt(&self, receipt: fabric::CapabilityTerminalReceipt) {
        self.items
            .lock()
            .await
            .push(ItemPayload::CapabilityReceipt {
                receipt: receipt.clone(),
            });
        self.inner.record_capability_receipt(receipt).await;
    }
    async fn record_model_context_projection(
        &self,
        mut receipt: fabric::model_projection::ModelContextProjectionReceipt,
    ) {
        materialize_projection_artifacts(&mut receipt);
        self.items
            .lock()
            .await
            .push(ItemPayload::ModelContextProjection {
                receipt: receipt.clone(),
            });
        self.inner.record_model_context_projection(receipt).await;
    }
    fn llm_provider(&self) -> Option<&dyn fabric::LlmProvider> {
        self.inner.llm_provider()
    }
    fn tool_definitions(&self) -> Vec<fabric::ToolDefinition> {
        self.inner.tool_definitions()
    }
    fn seed_messages(&self, request: &TurnRequest) -> Vec<fabric::Message> {
        let mut seed = self.inner.seed_messages(request);
        seed.extend(self.canonical_seed.clone());
        seed
    }
    fn turn_requirements(&self, request: &TurnRequest) -> Vec<fabric::TurnRequirement> {
        self.inner.turn_requirements(request)
    }

    async fn plan_capability_batch(
        &self,
        calls: Vec<CapabilityCall>,
    ) -> anyhow::Result<fabric::CapabilityBatchPlan> {
        self.inner.plan_capability_batch(calls).await
    }
}

pub(crate) fn materialize_projection_artifacts(
    receipt: &mut fabric::model_projection::ModelContextProjectionReceipt,
) {
    let store = corpus::tools::artifact::ArtifactStore::new(
        corpus::tools::tools::output::OutputConfig::default()
            .overflow_dir
            .join("artifacts"),
    );
    for fragment in &mut receipt.fragments {
        let Some(content) = fragment.inline_content.take() else {
            continue;
        };
        match store.store(content.as_bytes(), "application/json") {
            Ok(artifact) => fragment.artifact_ref = Some(artifact.uri()),
            Err(error) => {
                tracing::warn!(%error, fragment = %fragment.fragment_id, "context projection artifact persistence failed");
                fragment.inline_content = Some(content);
            }
        }
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;

    #[tokio::test]
    async fn recording_services_persist_terminal_receipt_as_distinct_item() {
        let services = RecordingTurnServices::new(Arc::new(fabric::StubTurnServices), Vec::new());
        let receipt = fabric::CapabilityTerminalReceipt {
            invocation_id: "call".into(),
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            capability: "validation_run".into(),
            status: fabric::CapabilityTerminalStatus::Succeeded,
            started_at: fabric::MonoTime(1),
            finished_at: fabric::MonoTime(2),
            exit_code: Some(0),
            error_class: None,
            artifact_ids: Vec::new(),
            evidence_ids: vec!["evidence".into()],
            output_ref: Some("command-session:id".into()),
            truncated: false,
            retry_disposition: fabric::CapabilityRetryDisposition::Never,
            audit_id: None,
        };

        services.record_capability_receipt(receipt.clone()).await;

        let items = services.take_items().await;
        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ItemPayload::CapabilityReceipt { receipt: stored } if stored == &receipt
        ));
    }

    #[tokio::test]
    async fn recording_services_persist_model_projection_as_control_item() {
        let services = RecordingTurnServices::new(Arc::new(fabric::StubTurnServices), Vec::new());
        let receipt = fabric::model_projection::ModelContextProjectionReceipt {
            inference_id: "inference-1".into(),
            operation_id: "operation-1".into(),
            role: "worker".into(),
            stage: "execute".into(),
            task_node_id: Some("task-1".into()),
            fragments: vec![fabric::model_projection::ModelContextFragmentReceipt {
                fragment_id: "fragment-1".into(),
                source: "system_prompt".into(),
                source_version: "version-1".into(),
                artifact_ref: None,
                inline_content: Some("{\"role\":\"system\"}".into()),
                selection_reason: "role_instruction".into(),
                classification: fabric::model_projection::ModelContextClassification::Instruction,
                truncated: false,
                bytes: 17,
            }],
            omitted_fragment_ids: Vec::new(),
            message_bytes: 10,
            tool_schema_bytes: 20,
        };

        services
            .record_model_context_projection(receipt.clone())
            .await;

        let items = services.take_items().await;
        let [ItemPayload::ModelContextProjection { receipt: stored }] = items.as_slice() else {
            panic!("projection receipt was not persisted as a distinct item")
        };
        assert!(stored.fragments[0].inline_content.is_none());
        assert!(stored.fragments[0]
            .artifact_ref
            .as_deref()
            .is_some_and(|reference| reference.starts_with("artifact://sha256/")));
    }
}
