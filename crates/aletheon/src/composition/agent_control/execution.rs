use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentHandle, AgentResult, AgentRunStatus,
    AgentSpawnRequest, AgoraSpaceId, EnvelopeV2, NamespaceId, ProcessId,
};
use async_trait::async_trait;
use runtime::{
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility,
    UnsequencedEvent,
};
use tokio_util::sync::CancellationToken;

use runtime::AgentContextProjection;
use runtime::{AgentRuntimeInbox, RuntimeProcessRegistrationPort};

pub use runtime::BackgroundResourceRegistration;

/// Compatibility aliases for the Runtime-owned Agent execution event
/// contract. The durable Runtime AgentStream sink has a separate name and
/// schema; these events are host observations consumed by this adapter.
pub use runtime::{
    AgentRecoveryRuntimeInput, AgentRuntimeEvent, AgentRuntimeEventSink as AgentEventSink,
};

pub use runtime::agent_admission::CognitiveTaskAdmissionPort;

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
    projections: Arc<dyn runtime::read_model::EventProjectionSink>,
}

impl SpineAgentEventSink {
    pub fn new(
        downstream: Arc<dyn AgentEventSink>,
        spine: Arc<dyn EventSpine>,
        input: AgentRuntimeInput,
        projections: Arc<dyn runtime::read_model::EventProjectionSink>,
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
            AgentRuntimeEvent::CapabilityAttenuated { report, .. } => (
                ::contracts::SchemaId::EVENT_AGENT_CAPABILITY_ATTENUATED_V1,
                "capability_attenuated",
                serde_json::to_value(report).unwrap_or(serde_json::Value::Null),
            ),
            AgentRuntimeEvent::Started { .. } => (
                ::contracts::SchemaId::EVENT_AGENT_STARTED_V1,
                "started",
                serde_json::Value::Null,
            ),
            AgentRuntimeEvent::Progress { summary, .. } => (
                ::contracts::SchemaId::EVENT_AGENT_PROGRESS_V1,
                "progress",
                serde_json::json!({"summary": summary.chars().take(4096).collect::<String>()}),
            ),
            AgentRuntimeEvent::Tool { name, is_error, .. } => (
                ::contracts::SchemaId::EVENT_TOOL_OBSERVATION_V1,
                "tool",
                serde_json::json!({"name": name, "is_error": is_error}),
            ),
            AgentRuntimeEvent::Terminal { status, result, .. } => (
                if *status == AgentRunStatus::Failed {
                    ::contracts::SchemaId::EVENT_AGENT_FAILED_V1
                } else {
                    ::contracts::SchemaId::EVENT_AGENT_STOPPED_V1
                },
                "terminal",
                serde_json::json!({
                    "status": format!("{status:?}"),
                    "turn_terminal_status": status.turn_terminal_status(),
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
            ::contracts::SchemaId(schema.into()),
            ::contracts::EnvelopeV2Target(format!("agent:{}", self.input.handle.agent_id.0)),
            ::contracts::EnvelopeV2Target(format!("agent-tree:{root}")),
            ::contracts::EnvelopeV2Delivery::FanOut,
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
    pub workspace: Option<::contracts::WorkspacePolicy>,
    /// Effective authority this Agent may pass to descendants. It is minted
    /// by AgentControl after intersecting the target profile with the parent
    /// authority and is distinct from the Agent's own callable tools.
    pub delegation_authority: ::contracts::AgentDelegationAuthority,
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
    /// Host-owned durable binding for an OS child. Runtime adapters may report
    /// identity, but cannot choose the logical Agent/process generation.
    pub runtime_process: Arc<dyn RuntimeProcessRegistrationPort>,
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
        HashMap<String, Arc<tokio::sync::RwLock<::contracts::EnvelopeV2Target>>>,
}

impl AgentRuntimeInput {
    pub fn candidate_projection_context(
        &self,
    ) -> application::agent::AgentCandidateProjectionContext {
        application::agent::AgentCandidateProjectionContext {
            handle: self.handle.clone(),
            workspace_id: self.workspace_id.clone(),
            root_workspace_id: self.root_workspace_id.clone(),
            root_process_id: self.root_process_id,
            broadcast_refs: self.request.broadcast_refs.clone(),
        }
    }

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
    ) -> Option<::contracts::EnvelopeV2Target> {
        let target = self.background_notification_targets.get(resource_id)?;
        Some(target.read().await.clone())
    }
}

fn runtime_error(message: &str) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: message.into(),
    }
}

#[async_trait]
pub trait AgentRuntimeLauncher: Send + Sync {
    fn resource_requirements(&self) -> runtime::RuntimeResourceRequirements {
        runtime::RuntimeResourceRequirements::default()
    }

    fn resumability(&self) -> ::contracts::RuntimeResumability {
        ::contracts::RuntimeResumability::Never
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

pub struct DelegateTaskRuntimeLauncher {
    backend: Arc<dyn runtime::DelegateTaskBackend>,
}

impl DelegateTaskRuntimeLauncher {
    pub fn new(backend: Arc<dyn runtime::DelegateTaskBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl AgentRuntimeLauncher for DelegateTaskRuntimeLauncher {
    fn resource_requirements(&self) -> runtime::RuntimeResourceRequirements {
        self.backend.resource_requirements()
    }

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
        let context = runtime::DelegateExecutionContext {
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            session_id: input.handle.root_agent_id.0.to_string(),
            working_dir: std::env::current_dir().unwrap_or_default(),
        };
        let result = self
            .backend
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
