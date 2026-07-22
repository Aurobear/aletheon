use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentHandle, AgentId, AgentResult, AgentRunStatus,
    AgentSpawnRequest, AgoraSpaceId, EnvelopeV2, EventId, EventIdentity, EventPayload, EventSpine,
    EventTreeId, EventVisibility, NamespaceId, OperationId, ProcessId, RuntimeId, UnsequencedEvent,
};
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::core::sub_agent::{SubAgentExecutionContext, SubAgentRuntime};

use super::context_fork::AgentContextProjection;
use super::mailbox::AgentRuntimeInbox;

#[derive(Debug, Clone)]
pub enum AgentRuntimeEvent {
    Started {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
    },
    Progress {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        summary: String,
    },
    Tool {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        name: String,
        is_error: bool,
    },
    Terminal {
        agent_id: AgentId,
        process_id: ProcessId,
        operation_id: OperationId,
        status: AgentRunStatus,
        result: Option<AgentResult>,
    },
}

#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentRuntimeEvent);
}

#[derive(Debug, Default)]
pub struct NoopAgentEventSink;

#[async_trait]
impl AgentEventSink for NoopAgentEventSink {
    async fn emit(&self, _event: AgentRuntimeEvent) {}
}

pub struct SpineAgentEventSink {
    downstream: Arc<dyn AgentEventSink>,
    spine: Arc<dyn EventSpine>,
    input: AgentRuntimeInput,
    projections: Arc<dyn crate::application::event_projection::EventProjectionSink>,
}

impl SpineAgentEventSink {
    pub fn new(
        downstream: Arc<dyn AgentEventSink>,
        spine: Arc<dyn EventSpine>,
        input: AgentRuntimeInput,
        projections: Arc<dyn crate::application::event_projection::EventProjectionSink>,
    ) -> Self {
        Self {
            downstream,
            spine,
            input,
            projections,
        }
    }

    fn append(&self, event: &AgentRuntimeEvent) -> anyhow::Result<()> {
        let (schema, kind, extra) = match event {
            AgentRuntimeEvent::Started { .. } => (
                fabric::SchemaId::EVENT_AGENT_STARTED_V1,
                "started",
                serde_json::Value::Null,
            ),
            AgentRuntimeEvent::Progress { summary, .. } => (
                fabric::SchemaId::TURN_EVENT_V1,
                "progress",
                serde_json::json!({"summary": summary.chars().take(4096).collect::<String>()}),
            ),
            AgentRuntimeEvent::Tool { name, is_error, .. } => (
                fabric::SchemaId::EVENT_TOOL_OBSERVATION_V1,
                "tool",
                serde_json::json!({"name": name, "is_error": is_error}),
            ),
            AgentRuntimeEvent::Terminal { status, result, .. } => (
                if *status == AgentRunStatus::Failed {
                    fabric::SchemaId::EVENT_AGENT_FAILED_V1
                } else {
                    fabric::SchemaId::EVENT_AGENT_STOPPED_V1
                },
                "terminal",
                serde_json::json!({
                    "status": format!("{status:?}"),
                    "has_result": result.is_some(),
                }),
            ),
        };
        let payload = serde_json::json!({
            "kind": kind,
            "agent_id": self.input.handle.agent_id.0,
            "process_id": self.input.handle.process_id.0,
            "operation_id": self.input.handle.operation_id.0,
            "root_agent_id": self.input.handle.root_agent_id.0,
            "parent_agent_id": self.input.handle.parent_agent_id.map(|id| id.0),
            "detail": extra,
        });
        let root = self.input.handle.root_agent_id.0.to_string();
        let mut envelope = EnvelopeV2::new(
            fabric::SchemaId(schema.into()),
            fabric::EnvelopeV2Target(format!("agent:{}", self.input.handle.agent_id.0)),
            fabric::EnvelopeV2Target(format!("agent-tree:{root}")),
            fabric::EnvelopeV2Delivery::FanOut,
            NamespaceId(format!("agent-tree:{root}")),
            payload.clone(),
        );
        envelope = envelope.with_operation_id(self.input.handle.operation_id);
        let event = self.spine.append(UnsequencedEvent {
            tree_id: EventTreeId::for_root_session(&root),
            event_id: EventId::new(),
            parent: None,
            identity: EventIdentity {
                root_session_id: root.clone(),
                session_id: root,
                agent_id: Some(self.input.handle.agent_id.0.to_string()),
            },
            envelope,
            visibility: EventVisibility::Control,
            payload: EventPayload::Inline { value: payload },
        })?;
        let report = self.projections.project(&event);
        for lag in report.lags.iter().filter(|lag| lag.pending_events > 0) {
            tracing::warn!(
                projection = %lag.projection,
                pending_events = lag.pending_events,
                "Agent event projection is behind its input watermark"
            );
        }
        for poison in &report.poisons {
            tracing::warn!(
                projection = %poison.projection,
                event_id = %poison.event_id,
                sequence = poison.sequence,
                "Agent event projection poison recorded"
            );
        }
        for failure in report.failures {
            tracing::warn!(
                projection = %failure.projection,
                error = %failure.error,
                "Agent event projection failed; unrelated reducers continued"
            );
        }
        Ok(())
    }
}

#[async_trait]
impl AgentEventSink for SpineAgentEventSink {
    async fn emit(&self, event: AgentRuntimeEvent) {
        if let Err(error) = self.append(&event) {
            tracing::warn!(%error, "canonical Agent event append rejected");
        }
        self.downstream.emit(event).await;
    }
}

#[derive(Debug, Clone)]
pub struct AgentRuntimeInput {
    pub request: AgentSpawnRequest,
    /// Workspace authority injected by the host capability boundary, never by
    /// model JSON.
    pub workspace: Option<fabric::WorkspacePolicy>,
    pub handle: AgentHandle,
    pub workspace_id: AgoraSpaceId,
    /// Root conscious workspace. Child-private candidates never use this
    /// space; explicitly exportable candidates are admitted here for a later
    /// C01 selection cycle.
    pub root_workspace_id: AgoraSpaceId,
    pub root_process_id: ProcessId,
    pub context: AgentContextProjection,
    /// Trusted process-bound memory authority derived by AgentControl.
    pub memory_context: mnemosyne::AgentMemoryContext,
    pub inbox: AgentRuntimeInbox,
    pub cancellation: CancellationToken,
    /// Per-declaration cancellation authority for background command
    /// producers. Producers must select by the reviewed resource ID instead
    /// of deriving an unmanaged token from the whole agent scope.
    pub background_cancellations: HashMap<String, CancellationToken>,
    /// Host-only producer registrations keyed by the reviewed declaration ID.
    /// This binds a real cancellation-aware future to settlement; it does not
    /// expose command text or declaration mutation to the model runtime.
    pub background_registrations: HashMap<String, BackgroundResourceRegistration>,
    /// Mutable host-owned notification destinations. A producer reads the
    /// current target for each emission; settlement may atomically switch it
    /// from the child mailbox to its parent mailbox.
    pub background_notification_targets:
        HashMap<String, Arc<tokio::sync::RwLock<fabric::EnvelopeV2Target>>>,
}

impl AgentRuntimeInput {
    pub fn background_cancellation(&self, resource_id: &str) -> Option<CancellationToken> {
        self.background_cancellations.get(resource_id).cloned()
    }

    /// Bind exactly one host-created producer future to a reviewed resource.
    pub fn register_background_producer<F, Fut>(
        &self,
        resource_id: &str,
        producer: F,
    ) -> Result<(), AgentControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let registration = self
            .background_registrations
            .get(resource_id)
            .ok_or_else(|| {
                runtime_error("background producer has no reviewed resource declaration")
            })?;
        registration.bind(producer)
    }

    pub async fn background_notification_target(
        &self,
        resource_id: &str,
    ) -> Option<fabric::EnvelopeV2Target> {
        let target = self.background_notification_targets.get(resource_id)?;
        Some(target.read().await.clone())
    }
}

const REGISTRATION_UNBOUND: u8 = 0;
const REGISTRATION_RUNNING: u8 = 1;
const REGISTRATION_STOPPED: u8 = 2;

#[derive(Clone, Debug)]
pub struct BackgroundResourceRegistration {
    token: CancellationToken,
    state: Arc<AtomicU8>,
    stopped: Arc<tokio::sync::Notify>,
}

impl BackgroundResourceRegistration {
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self {
            token,
            state: Arc::new(AtomicU8::new(REGISTRATION_UNBOUND)),
            stopped: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn bind<F, Fut>(&self, producer: F) -> Result<(), AgentControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.state
            .compare_exchange(
                REGISTRATION_UNBOUND,
                REGISTRATION_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| runtime_error("background producer is already registered"))?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            self.state.store(REGISTRATION_UNBOUND, Ordering::Release);
            runtime_error("background producer registration requires a Tokio runtime")
        })?;
        let token = self.token.clone();
        let producer_token = token.clone();
        let state = self.state.clone();
        let stopped = self.stopped.clone();
        runtime.spawn(async move {
            let producer = producer(producer_token);
            tokio::pin!(producer);
            tokio::select! {
                _ = &mut producer => {}
                _ = token.cancelled() => {
                    // Give the cancellation-aware producer a bounded grace
                    // period to reap its child/process resources itself.
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        &mut producer,
                    ).await;
                }
            }
            state.store(REGISTRATION_STOPPED, Ordering::Release);
            stopped.notify_waiters();
        });
        Ok(())
    }

    pub(crate) async fn cancel_and_wait(&self) {
        self.token.cancel();
        self.wait_stopped().await;
    }

    pub async fn wait_stopped(&self) {
        loop {
            let notified = self.stopped.notified();
            if self.state.load(Ordering::Acquire) != REGISTRATION_RUNNING {
                return;
            }
            notified.await;
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::Acquire) == REGISTRATION_STOPPED
    }
}

fn runtime_error(message: &str) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
pub struct AgentRecoveryRuntimeInput {
    pub handle: AgentHandle,
    pub request: AgentSpawnRequest,
    pub checkpoint_reference: String,
}

#[async_trait]
pub trait AgentRuntimeLauncher: Send + Sync {
    fn resumability(&self) -> fabric::RuntimeResumability {
        fabric::RuntimeResumability::Never
    }

    async fn resume_from_checkpoint(
        &self,
        _input: AgentRecoveryRuntimeInput,
    ) -> Result<(), AgentControlError> {
        Err(AgentControlError {
            kind: AgentControlErrorKind::Runtime,
            message: "runtime does not implement checkpoint resume".into(),
        })
    }

    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError>;
}

#[derive(Default)]
pub struct AgentRuntimeRegistry {
    runtimes: RwLock<HashMap<RuntimeId, Arc<dyn AgentRuntimeLauncher>>>,
    manifests: RwLock<HashMap<RuntimeId, runtime::RuntimeManifest>>,
}

impl AgentRuntimeRegistry {
    pub fn register(
        &self,
        id: RuntimeId,
        launcher: Arc<dyn AgentRuntimeLauncher>,
    ) -> Result<(), AgentControlError> {
        if id.0.trim().is_empty() {
            return Err(AgentControlError::invalid("runtime id must not be empty"));
        }
        let mut runtimes = self.runtimes.write();
        if runtimes.contains_key(&id) {
            return Err(AgentControlError {
                kind: AgentControlErrorKind::Conflict,
                message: format!("runtime already registered: {}", id.0),
            });
        }
        runtimes.insert(id, launcher);
        Ok(())
    }

    /// Register a selectable runtime contract alongside the Executive-owned
    /// launcher. The manifest describes capabilities; it does not gain
    /// lifecycle, admission, cancellation, or settlement authority.
    pub fn register_manifested(
        &self,
        id: RuntimeId,
        launcher: Arc<dyn AgentRuntimeLauncher>,
        manifest: runtime::RuntimeManifest,
    ) -> Result<(), AgentControlError> {
        if manifest.id != id.0 {
            return Err(AgentControlError::invalid(
                "runtime manifest id differs from registry id",
            ));
        }
        self.register(id.clone(), launcher)?;
        self.manifests.write().insert(id, manifest);
        Ok(())
    }

    pub fn resolve_selector(
        &self,
        selector: &runtime::RuntimeSelector,
        required: &[runtime::RuntimeCapability],
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError> {
        let manifests = self.manifests.read();
        let id = selector
            .resolve_id(manifests.values(), required)
            .map_err(|message| AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message,
            })?;
        drop(manifests);
        self.resolve(&RuntimeId(id))
    }

    pub fn resolve(
        &self,
        id: &RuntimeId,
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError> {
        self.runtimes
            .read()
            .get(id)
            .cloned()
            .ok_or_else(|| AgentControlError {
                kind: AgentControlErrorKind::NotFound,
                message: format!("runtime is not registered: {}", id.0),
            })
    }
}

pub struct CompatibilityRuntimeLauncher {
    runtime: Arc<dyn SubAgentRuntime>,
}

impl CompatibilityRuntimeLauncher {
    pub fn new(runtime: Arc<dyn SubAgentRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl AgentRuntimeLauncher for CompatibilityRuntimeLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        events
            .emit(AgentRuntimeEvent::Started {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
            })
            .await;
        let context = SubAgentExecutionContext {
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            session_id: input.handle.root_agent_id.0.to_string(),
            working_dir: std::env::current_dir().unwrap_or_default(),
        };
        let result = self
            .runtime
            .run_attempt_in_context(&input.request.task, input.cancellation, context)
            .await
            .map_err(|failure| AgentControlError {
                kind: AgentControlErrorKind::Runtime,
                message: failure.message,
            })?;
        let result = AgentResult {
            output: result.output,
            usage: result.usage,
            evidence: result.evidence,
            artifacts: vec![],
        };
        result.validate()?;
        events
            .emit(AgentRuntimeEvent::Terminal {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
                status: AgentRunStatus::Succeeded,
                result: Some(result.clone()),
            })
            .await;
        Ok(result)
    }
}
