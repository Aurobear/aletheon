use crate::r#impl::session::canonical_store::CanonicalSessionStore;
use crate::service::harness_factory::{CognitiveSessionFactory, LinearCognitiveSessionFactory};
use crate::service::turn_coordinator::{cancelled_result, TurnCoordinator, TurnExecution};
use crate::service::turn_policy::TurnPolicy;
use crate::service::{PostTurnPipeline, PreTurnPipeline};
use aletheon_kernel::chronos::SystemTimer;
use aletheon_kernel::KernelRuntime;
use anyhow::Result;
use async_trait::async_trait;
use cognit::harness::HarnessConfig;
use fabric::{
    CapabilityCall, CapabilityResult, Clock, ItemPayload, RecallRequest, RecallSet, Timer,
    TurnEventSink, TurnRequest, TurnServices,
};
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
        let coordinator = Arc::new(TurnCoordinator::new(kernel.clone(), store));
        Self {
            services,
            pre_turn,
            post_turn,
            factory: Arc::new(LinearCognitiveSessionFactory::new(HarnessConfig::default())),
            clock: kernel.clock(),
            coordinator,
            policy: TurnPolicy::exec(),
        }
    }

    pub fn with_harness_config(mut self, harness_config: HarnessConfig) -> Self {
        self.factory = Arc::new(LinearCognitiveSessionFactory::new(harness_config));
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
                    id: fabric::SessionId(request.session_id.clone()),
                    parent: None,
                    created_at_ms: 0,
                    status: fabric::SessionStatus::Active,
                };
                let mut history = history_store
                    .load_items(&fabric::SessionId(request.session_id.clone()), None)
                    .await?;
                if history.last().is_some_and(|item| {
                    matches!(&item.payload, ItemPayload::UserMessage { content } if content == &request.input)
                }) {
                    history.pop();
                }
                let canonical_seed = crate::r#impl::session::canonical_store::project_messages(&history)?;
                let recording = RecordingTurnServices::new(services, canonical_seed);
                let request = pre_turn.run(request, &recording).await?;
                let mut session = factory.create(&session_record, &runner_policy).await?;
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
}
