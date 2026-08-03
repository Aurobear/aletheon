//! Object-safe cognitive session adapter used by the executive turn service.

use crate::core::{
    AgentRuntimeId, CognitiveTaskContract, CognitiveTaskKind, CognitiveTurnState,
    CompletionGateMode, RequiredAction, ValidationRequirement,
};
use crate::harness::config::HarnessConfig;
use crate::harness::linear::DynLlmRef;
use crate::harness::linear::{BatchPlanner, CompactorTrait, ReActLoop};
use async_trait::async_trait;
use fabric::types::inference_receipt::{
    InferenceTerminalReceipt, InferenceTerminalStatus, INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
};
use fabric::{
    CapabilityCall, CapabilityErrorClass, CapabilityReceiptDetails, CapabilityRetryDisposition,
    CapabilityTerminalReceipt, CapabilityTerminalStatus, Message, TurnEvent, TurnEventSink,
    TurnMetrics as FabricTurnMetrics, TurnRequest, TurnResult, TurnServices, TurnStop,
};
use sha2::{Digest, Sha256};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_DASEIN_CONTEXT_BYTES: usize = 8_000;

struct ProjectionRecordingLlm<'a> {
    inner: &'a dyn fabric::LlmProvider,
    services: &'a dyn TurnServices,
    operation_id: fabric::OperationId,
    pending: Arc<Mutex<Vec<InferenceTerminalReceipt>>>,
}

impl ProjectionRecordingLlm<'_> {
    async fn record(
        &self,
        messages: &[Message],
        tools: &[fabric::ToolDefinition],
    ) -> anyhow::Result<InferenceMetadata> {
        use fabric::model_projection::{
            ModelContextClassification, ModelContextFragmentReceipt, ModelContextProjectionReceipt,
        };
        let fragments = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let bytes = serde_json::to_vec(message).unwrap_or_default();
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let classification = if message
                    .content
                    .iter()
                    .any(|block| matches!(block, fabric::ContentBlock::ToolResult { .. }))
                {
                    ModelContextClassification::ToolEvidence
                } else {
                    match message.role {
                        fabric::Role::System => ModelContextClassification::Instruction,
                        fabric::Role::User => ModelContextClassification::UntrustedInput,
                        fabric::Role::Assistant => ModelContextClassification::ModelHistory,
                    }
                };
                ModelContextFragmentReceipt {
                    fragment_id: digest.clone(),
                    source: format!("message:{index}:{:?}", message.role),
                    source_version: digest,
                    artifact_ref: None,
                    inline_content: Some(String::from_utf8_lossy(&bytes).into_owned()),
                    selection_reason: "active_turn_context".into(),
                    classification,
                    truncated: false,
                    bytes: bytes.len() as u64,
                }
            })
            .collect::<Vec<_>>();
        let message_bytes = fragments.iter().map(|fragment| fragment.bytes).sum();
        let tool_schema_bytes = serde_json::to_vec(tools)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or_default();
        let inference_id = uuid::Uuid::new_v4().to_string();
        let system_bytes = serde_json::to_vec(
            &messages
                .iter()
                .filter(|message| message.role == fabric::Role::System)
                .collect::<Vec<_>>(),
        )?;
        let system_prefix_digest = format!("sha256:{:x}", Sha256::digest(system_bytes));
        let tool_schema_digest = fabric::tool_schema_digest(tools)?;
        self.services
            .record_model_context_projection(ModelContextProjectionReceipt {
                inference_id: inference_id.clone(),
                operation_id: format!("{:?}", self.operation_id),
                system_prefix_digest: system_prefix_digest.clone(),
                tool_schema_digest: tool_schema_digest.clone(),
                role: "active_agent".into(),
                stage: "cognitive_loop".into(),
                task_node_id: Some(format!("{:?}", self.operation_id)),
                fragments,
                omitted_fragment_ids: Vec::new(),
                message_bytes,
                tool_schema_bytes,
            })
            .await;
        let facts = self.inner.runtime_facts();
        Ok(InferenceMetadata {
            inference_id,
            operation_id: format!("{:?}", self.operation_id),
            provider_id: self.inner.name().to_owned(),
            model_id: facts.effective_model_id,
            system_prefix_digest,
            tool_schema_digest,
        })
    }

    async fn drain(&self) {
        let receipts = std::mem::take(&mut *self.pending.lock().expect("receipt queue poisoned"));
        for receipt in receipts {
            self.services.record_inference_receipt(receipt).await;
        }
    }
}

#[derive(Clone)]
struct InferenceMetadata {
    inference_id: String,
    operation_id: String,
    provider_id: String,
    model_id: String,
    system_prefix_digest: String,
    tool_schema_digest: String,
}

impl InferenceMetadata {
    fn receipt(
        &self,
        status: InferenceTerminalStatus,
        usage: fabric::InferenceUsage,
        failure_kind: Option<&str>,
    ) -> InferenceTerminalReceipt {
        InferenceTerminalReceipt {
            schema_version: INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
            inference_id: self.inference_id.clone(),
            operation_id: self.operation_id.clone(),
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
            system_prefix_digest: self.system_prefix_digest.clone(),
            tool_schema_digest: self.tool_schema_digest.clone(),
            status,
            usage,
            failure_kind: failure_kind.map(str::to_owned),
        }
    }
}

struct TerminalRecordingStream {
    inner: fabric::LlmStream,
    metadata: InferenceMetadata,
    pending: Arc<Mutex<Vec<InferenceTerminalReceipt>>>,
    usage: fabric::InferenceUsage,
    terminal: bool,
}

impl TerminalRecordingStream {
    fn finish(&mut self, status: InferenceTerminalStatus, failure_kind: Option<&str>) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.pending
            .lock()
            .expect("receipt queue poisoned")
            .push(
                self.metadata
                    .receipt(status, self.usage.clone(), failure_kind),
            );
    }
}

impl futures::Stream for TerminalRecordingStream {
    type Item = anyhow::Result<fabric::StreamChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(fabric::StreamChunk::Usage { usage }))) => {
                self.usage = usage.clone();
                Poll::Ready(Some(Ok(fabric::StreamChunk::Usage { usage })))
            }
            Poll::Ready(Some(Ok(fabric::StreamChunk::Done { stop_reason }))) => {
                self.finish(InferenceTerminalStatus::Succeeded, None);
                Poll::Ready(Some(Ok(fabric::StreamChunk::Done { stop_reason })))
            }
            Poll::Ready(Some(Err(error))) => {
                self.finish(InferenceTerminalStatus::Failed, Some("unknown"));
                Poll::Ready(Some(Err(error)))
            }
            other => other,
        }
    }
}

impl Drop for TerminalRecordingStream {
    fn drop(&mut self) {
        self.finish(InferenceTerminalStatus::Cancelled, Some("cancelled"));
    }
}

#[async_trait]
impl fabric::LlmProvider for ProjectionRecordingLlm<'_> {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[fabric::ToolDefinition],
    ) -> anyhow::Result<fabric::LlmResponse> {
        let tools = fabric::canonicalize_tool_definitions(tools)?;
        let metadata = self.record(messages, &tools).await?;
        let result = self.inner.complete(messages, &tools).await;
        let receipt = match &result {
            Ok(response) => metadata.receipt(
                InferenceTerminalStatus::Succeeded,
                response.usage.clone(),
                None,
            ),
            Err(_) => metadata.receipt(
                InferenceTerminalStatus::Failed,
                fabric::InferenceUsage::default(),
                Some("unknown"),
            ),
        };
        self.services.record_inference_receipt(receipt).await;
        result
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[fabric::ToolDefinition],
    ) -> anyhow::Result<fabric::LlmStream> {
        let tools = fabric::canonicalize_tool_definitions(tools)?;
        let metadata = self.record(messages, &tools).await?;
        match self.inner.complete_stream(messages, &tools).await {
            Ok(inner) => Ok(Box::pin(TerminalRecordingStream {
                inner,
                metadata,
                pending: self.pending.clone(),
                usage: fabric::InferenceUsage::default(),
                terminal: false,
            })),
            Err(error) => {
                self.services
                    .record_inference_receipt(metadata.receipt(
                        InferenceTerminalStatus::Failed,
                        fabric::InferenceUsage::default(),
                        Some("unknown"),
                    ))
                    .await;
                Err(error)
            }
        }
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn runtime_facts(&self) -> fabric::ModelRuntimeFacts {
        self.inner.runtime_facts()
    }

    fn max_context_length(&self) -> usize {
        self.inner.max_context_length()
    }
}

pub type CognitiveStreamEvent = crate::harness::event_sink::Event;

/// Cognit-owned streaming event boundary used by interactive frontends.
pub trait CognitiveStreamSink: Send + Sync {
    fn emit(&self, event: CognitiveStreamEvent);
}

pub struct ChannelCognitiveStreamSink {
    tx: tokio::sync::mpsc::Sender<CognitiveStreamEvent>,
}

/// Production sink that writes Cognit lifecycle events directly onto the
/// canonical Fabric turn stream.
pub struct CanonicalTurnEventSink {
    sender: fabric::ipc::TurnEventSender,
}

impl CanonicalTurnEventSink {
    pub fn new(sender: fabric::ipc::TurnEventSender) -> Self {
        Self { sender }
    }
}

impl CognitiveStreamSink for CanonicalTurnEventSink {
    fn emit(&self, event: CognitiveStreamEvent) {
        let _ = self.sender.send(&event.into());
    }
}

impl ChannelCognitiveStreamSink {
    pub fn new(tx: tokio::sync::mpsc::Sender<CognitiveStreamEvent>) -> Self {
        Self { tx }
    }
}

impl CognitiveStreamSink for ChannelCognitiveStreamSink {
    fn emit(&self, event: CognitiveStreamEvent) {
        let _ = self.tx.try_send(event);
    }
}

struct CognitiveStreamAdapter<'a>(&'a dyn CognitiveStreamSink);

impl crate::harness::event_sink::EventSink for CognitiveStreamAdapter<'_> {
    fn emit(&self, event: CognitiveStreamEvent) {
        self.0.emit(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CognitRetryDisposition {
    Never,
    AfterBackoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CognitErrorKind {
    Cancelled,
    ContextOverflow,
    TransientProvider,
    TerminalRuntime,
}

#[derive(Debug, Error)]
#[error("cognitive session {kind:?}: {message}")]
pub struct CognitError {
    kind: CognitErrorKind,
    message: String,
}

impl CognitError {
    pub fn kind(&self) -> CognitErrorKind {
        self.kind
    }

    pub const fn retry_disposition(&self) -> CognitRetryDisposition {
        match self.kind {
            CognitErrorKind::TransientProvider => CognitRetryDisposition::AfterBackoff,
            CognitErrorKind::Cancelled
            | CognitErrorKind::ContextOverflow
            | CognitErrorKind::TerminalRuntime => CognitRetryDisposition::Never,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            kind: CognitErrorKind::Cancelled,
            message: "turn cancellation requested".into(),
        }
    }

    pub fn terminal(message: impl Into<String>) -> Self {
        Self {
            kind: CognitErrorKind::TerminalRuntime,
            message: message.into(),
        }
    }

    fn from_runtime(error: anyhow::Error) -> Self {
        use crate::adapters::inference::scheduler::{classify_error, ErrorClass};
        let kind = match classify_error(&error) {
            ErrorClass::Transient => CognitErrorKind::TransientProvider,
            ErrorClass::ContextOverflow => CognitErrorKind::ContextOverflow,
            ErrorClass::Terminal => CognitErrorKind::TerminalRuntime,
        };
        Self {
            kind,
            message: bounded_error(&error.to_string()),
        }
    }
}

pub struct CognitiveSessionDependencies {
    pub clock: Arc<dyn fabric::Clock>,
    pub cancellation: CancellationToken,
    pub compactor: Option<Box<dyn CompactorTrait>>,
    pub batch_planner: Option<Arc<dyn BatchPlanner>>,
    /// Optional production bridge for messages evicted by guarded compaction.
    /// Absence is an explicit no-op; compaction metrics still record eviction.
    pub evicted_callback: Option<Arc<dyn Fn(Vec<Message>) + Send + Sync>>,
    /// Optional coding verifier (Wave 3). When set, ReActLoop validates the
    /// model's final answer before accepting it as complete.
    pub verifier: Option<Arc<dyn fabric::policy::verifier::Verifier>>,
    pub grounded_outcome_sink: Option<Arc<dyn crate::core::GroundedOutcomeSink>>,
}

struct NoopCompressor;

impl CompactorTrait for NoopCompressor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn crate::adapters::inference::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn crate::adapters::inference::provider::LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

#[async_trait]
pub trait CognitiveSession: Send {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError>;

    async fn run_streaming_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
        stream: &dyn CognitiveStreamSink,
    ) -> Result<TurnResult, CognitError> {
        let result = self.run_turn(request, services, events).await?;
        stream.emit(CognitiveStreamEvent::Text {
            text: result.output.clone(),
        });
        stream.emit(CognitiveStreamEvent::TurnDone {
            result: Ok(result.output.clone()),
        });
        Ok(result)
    }
}

pub struct LinearCognitiveSession {
    inner: ReActLoop,
    cancellation: CancellationToken,
    clock: Arc<dyn fabric::Clock>,
}

impl LinearCognitiveSession {
    pub fn new(config: HarnessConfig, dependencies: CognitiveSessionDependencies) -> Self {
        let compactor = dependencies
            .compactor
            .unwrap_or_else(|| Box::new(NoopCompressor));
        let clock = dependencies.clock;
        let mut inner = ReActLoop::new_with_clock(config, compactor, clock.clone());
        if let Some(planner) = dependencies.batch_planner.as_ref() {
            inner.set_batch_planner(Arc::clone(planner));
        }
        if let Some(sink) = dependencies.grounded_outcome_sink {
            inner.set_grounded_outcome_sink(sink);
        }
        if let Some(callback) = dependencies.evicted_callback {
            inner.set_evicted_callback(callback);
        }
        if let Some(verifier) = dependencies.verifier {
            inner.set_verifier(verifier);
        }
        Self {
            inner,
            cancellation: dependencies.cancellation,
            clock,
        }
    }

    /// Create a session wrapping a pre-built ReActLoop.
    ///
    /// Useful when the loop is constructed by a shared factory, e.g.
    /// `harness_factory::build_configured_react_loop()` in the daemon path.
    pub fn from_react_loop(inner: ReActLoop, cancellation: CancellationToken) -> Self {
        let clock = inner.clock_handle();
        Self {
            inner,
            cancellation,
            clock,
        }
    }

    fn configure_turn_contract(
        &mut self,
        request: &TurnRequest,
        services: &dyn TurnServices,
    ) -> Option<Message> {
        let requirements = services.turn_requirements(request);
        let evaluation = request.evaluation_contract.as_ref();
        if requirements.is_empty() && evaluation.is_none() {
            self.inner.clear_cognitive_state();
            return None;
        }
        let model_contract = render_turn_contract(evaluation, &requirements);
        let track_evaluation_obligations = evaluation
            .is_some_and(|contract| contract.mode == fabric::EvaluationMode::Enforce)
            || requirements.is_empty();
        let mut required_actions = requirements
            .iter()
            .cloned()
            .map(|requirement| match requirement {
                fabric::TurnRequirement::InvokeAgentRuntime { runtime_id } => {
                    RequiredAction::InvokeAgent {
                        runtime: AgentRuntimeId(runtime_id),
                    }
                }
                fabric::TurnRequirement::InvokeCapability { name } => {
                    RequiredAction::InvokeTool { tool_name: name }
                }
                fabric::TurnRequirement::ObserveTerminal { operation_id } => {
                    RequiredAction::ObserveTerminal { operation_id }
                }
                fabric::TurnRequirement::RunRoleGraph { .. } => RequiredAction::RunRoleGraph {
                    root_task_id: request
                        .context
                        .turn_id
                        .map(|turn_id| format!("root:{}", turn_id.0))
                        .unwrap_or_else(|| "root:unassigned".into()),
                    receipt_id: None,
                },
            })
            .collect::<Vec<_>>();
        let mut validation_requirements = Vec::new();
        if let Some(contract) = evaluation.filter(|_| track_evaluation_obligations) {
            for evidence in &contract.required_evidence {
                let evidence_kind = evidence_kind_name(evidence.kind);
                for index in 1..=evidence.minimum_count {
                    let sequence = (evidence.minimum_count > 1)
                        .then(|| format!(" (item {index}/{})", evidence.minimum_count))
                        .unwrap_or_default();
                    validation_requirements.push(ValidationRequirement {
                        id: format!("evaluation:evidence:{evidence_kind}:{index}"),
                        description: format!(
                            "observe {} `{evidence_kind}` evidence{sequence}",
                            if evidence.authoritative {
                                "authoritative"
                            } else {
                                "eligible"
                            }
                        ),
                    });
                }
                if evidence.kind
                    == fabric::types::metacognition_evidence::EvidenceKind::VerificationResult
                    && !required_actions.iter().any(|action| {
                        matches!(
                            action,
                            RequiredAction::InvokeTool { tool_name }
                                if tool_name == "validation_run"
                        )
                    })
                {
                    required_actions.push(RequiredAction::InvokeTool {
                        tool_name: "validation_run".into(),
                    });
                }
            }
        }
        let task_kind = if evaluation.is_some() {
            CognitiveTaskKind::CodeChange
        } else if required_actions
            .iter()
            .any(|action| matches!(action, RequiredAction::InvokeAgent { .. }))
        {
            CognitiveTaskKind::RequiredAgentExecution
        } else {
            CognitiveTaskKind::General
        };
        self.inner
            .set_cognitive_state(CognitiveTurnState::from_contract(CognitiveTaskContract {
                objective: request.input.clone(),
                task_kind,
                required_actions,
                deliverables: Vec::new(),
                validation_requirements,
            }));
        self.inner
            .set_completion_gate_mode(match (requirements.is_empty(), evaluation) {
                (true, Some(contract)) if contract.mode == fabric::EvaluationMode::Shadow => {
                    CompletionGateMode::Shadow
                }
                _ => CompletionGateMode::Enforce,
            });
        Some(Message::system(model_contract))
    }
}

fn render_turn_contract(
    evaluation: Option<&fabric::TaskEvaluationContract>,
    requirements: &[fabric::TurnRequirement],
) -> String {
    let mut lines = vec![
        "[cognitive_task_contract]".to_owned(),
        "These host-authored obligations apply to this turn; their typed modes determine whether they are observed or enforced at completion:"
            .to_owned(),
    ];
    if let Some(contract) = evaluation {
        lines.push(format!(
            "- Perform an explicitly typed coding task under rubric `{}` version {} in `{:?}` mode.",
            contract.rubric.0, contract.rubric_version, contract.mode
        ));
        for evidence in &contract.required_evidence {
            lines.push(format!(
                "- Produce at least {} {}{} evidence item(s).",
                evidence.minimum_count,
                if evidence.authoritative {
                    "authoritative "
                } else {
                    ""
                },
                evidence_kind_name(evidence.kind)
            ));
            if evidence.kind
                == fabric::types::metacognition_evidence::EvidenceKind::VerificationResult
            {
                lines.push(
                    "- Invoke `validation_run` and observe its authoritative terminal result; a pending command is not completion evidence."
                        .into(),
                );
            }
        }
        for gate in &contract.required_gates {
            lines.push(format!("- Satisfy evaluation gate `{}`.", gate.name));
        }
    }
    for requirement in requirements {
        lines.push(match requirement {
            fabric::TurnRequirement::InvokeAgentRuntime { runtime_id } => format!(
                "- During this turn call `agent_spawn` with its `runtime` field set exactly to `{runtime_id}` (the runtime ID is not a profile name), then call `agent_wait` for the returned `agent_id` and observe an authoritative terminal result whose `runtime_id` is `{runtime_id}`; historical Agent receipts do not satisfy this obligation."
            ),
            fabric::TurnRequirement::InvokeCapability { name } => {
                format!("- Invoke capability `{name}` during this turn.")
            }
            fabric::TurnRequirement::ObserveTerminal { operation_id } => format!(
                "- Observe authoritative terminal evidence for operation `{}`.",
                operation_id.0
            ),
            fabric::TurnRequirement::RunRoleGraph { .. } =>
                "- Complete the host-authorized canonical role graph and persist its terminal receipt.".into(),
        });
    }
    lines.join("\n")
}

fn evidence_kind_name(kind: fabric::types::metacognition_evidence::EvidenceKind) -> &'static str {
    use fabric::types::metacognition_evidence::EvidenceKind;
    match kind {
        EvidenceKind::Assertion => "assertion",
        EvidenceKind::Observation => "observation",
        EvidenceKind::ActionResult => "action_result",
        EvidenceKind::VerificationResult => "verification_result",
        EvidenceKind::Metric => "metric",
        EvidenceKind::Artifact => "artifact",
        EvidenceKind::HumanFeedback => "human_feedback",
        EvidenceKind::PolicyDecision => "policy_decision",
        EvidenceKind::RuntimeFault => "runtime_fault",
    }
}

async fn invoke_with_terminal_receipt(
    services: &dyn TurnServices,
    call: CapabilityCall,
    clock: &dyn fabric::Clock,
) -> fabric::CapabilityResult {
    let started_at = clock.mono_now();
    let result = services.invoke(call.clone()).await;
    let finished_at = clock.mono_now();
    if let Some(details) = terminal_receipt_details(&call.name, &result) {
        services
            .record_capability_receipt(CapabilityTerminalReceipt::from_terminal_result(
                &call,
                &result,
                started_at,
                finished_at,
                details,
            ))
            .await;
    }
    result
}

fn terminal_receipt_details(
    capability: &str,
    result: &fabric::CapabilityResult,
) -> Option<CapabilityReceiptDetails> {
    if !matches!(
        capability,
        "exec_command" | "write_stdin" | "validation_run"
    ) {
        return Some(CapabilityReceiptDetails::default());
    }

    let payload: serde_json::Value = serde_json::from_str(&result.output).ok()?;
    let terminal = payload.get("terminal")?;
    if terminal.is_null() {
        return None;
    }
    let status = match terminal.get("status").and_then(|value| value.as_str()) {
        Some("exited") if terminal.get("exit_code").and_then(|value| value.as_i64()) == Some(0) => {
            CapabilityTerminalStatus::Succeeded
        }
        Some("exited") | Some("failed") => CapabilityTerminalStatus::Failed,
        Some("timed_out") => CapabilityTerminalStatus::TimedOut,
        Some("cancelled") => CapabilityTerminalStatus::Cancelled,
        _ => return None,
    };
    let error_class = match status {
        CapabilityTerminalStatus::TimedOut => Some(CapabilityErrorClass::Timeout),
        CapabilityTerminalStatus::Failed => Some(CapabilityErrorClass::Unknown),
        CapabilityTerminalStatus::Succeeded | CapabilityTerminalStatus::Cancelled => None,
    };
    Some(CapabilityReceiptDetails {
        status: Some(status),
        exit_code: terminal
            .get("exit_code")
            .and_then(|value| value.as_i64())
            .and_then(|value| i32::try_from(value).ok()),
        error_class,
        output_ref: payload
            .get("output_artifact_ref")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                payload
                    .get("session_id")
                    .and_then(|value| value.as_str())
                    .map(|id| format!("command-session:{id}"))
            }),
        truncated: payload
            .get("truncated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        retry_disposition: if status == CapabilityTerminalStatus::Succeeded {
            CapabilityRetryDisposition::Never
        } else {
            CapabilityRetryDisposition::AfterCorrection
        },
        ..CapabilityReceiptDetails::default()
    })
}

#[async_trait]
impl CognitiveSession for LinearCognitiveSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;

        if self.cancellation.is_cancelled() {
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                })
                .await;
            return Err(CognitError::cancelled());
        }

        let result = if let Some(llm) = services.llm_provider() {
            self.inner.reset();
            let contract_message = self.configure_turn_contract(&request, services);
            let mut seed_messages = services.seed_messages(&request);
            if let Some(contract_message) = contract_message {
                seed_messages.push(contract_message);
            }
            if !seed_messages.is_empty() {
                self.inner.seed_messages(seed_messages);
            }
            let tool_defs = services.tool_definitions();
            let process_id = request.process_id;
            let clock = self.clock.clone();
            let recording_llm = ProjectionRecordingLlm {
                inner: llm,
                services,
                operation_id: request.operation_id,
                pending: Arc::new(Mutex::new(Vec::new())),
            };
            let llm = DynLlmRef(&recording_llm);
            let run = self
                .inner
                .run(&request.input, &llm, &tool_defs, |call_id, name, input| {
                    let clock = clock.clone();
                    let req = CapabilityCall {
                        operation_id: request.operation_id,
                        process_id,
                        name: name.to_string(),
                        input: input.clone(),
                        call_id: call_id.to_string(),
                        deadline: None,
                    };
                    async move {
                        let result =
                            invoke_with_terminal_receipt(services, req, clock.as_ref()).await;
                        (result.output, result.is_error)
                    }
                });
            let (output, metrics) = tokio::select! {
                _ = self.cancellation.cancelled() => {
                    recording_llm.drain().await;
                    events.emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Cancelled,
                    }).await;
                    return Err(CognitError::cancelled());
                }
                result = run => match result {
                    Ok(result) => result,
                    Err(error) => {
                        recording_llm.drain().await;
                        events.emit(TurnEvent::Finished {
                            operation_id: request.operation_id,
                            stop: TurnStop::Failed,
                        }).await;
                        return Err(CognitError::from_runtime(error));
                    }
                }
            };
            recording_llm.drain().await;
            TurnResult {
                output,
                stop: metrics.stop.clone(),
                metrics: FabricTurnMetrics {
                    tool_calls_made: metrics.tool_calls_made,
                    tool_errors: metrics.tool_errors,
                    provider_retries: metrics.provider_retries,
                    elapsed_ms: metrics.elapsed_ms,
                    iterations: metrics.iterations,
                    completed_normally: metrics.completed_normally,
                },
            }
        } else {
            TurnResult {
                output: request.input,
                stop: TurnStop::Completed,
                metrics: FabricTurnMetrics {
                    completed_normally: true,
                    ..Default::default()
                },
            }
        };

        events
            .emit(TurnEvent::Finished {
                operation_id: request.operation_id,
                stop: result.stop.clone(),
            })
            .await;
        Ok(result)
    }

    async fn run_streaming_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
        stream: &dyn CognitiveStreamSink,
    ) -> Result<TurnResult, CognitError> {
        events
            .emit(TurnEvent::Started {
                operation_id: request.operation_id,
            })
            .await;
        if self.cancellation.is_cancelled() {
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                })
                .await;
            return Err(CognitError::cancelled());
        }

        let Some(llm) = services.llm_provider() else {
            let result = TurnResult {
                output: request.input,
                stop: TurnStop::Completed,
                metrics: FabricTurnMetrics {
                    completed_normally: true,
                    ..Default::default()
                },
            };
            stream.emit(CognitiveStreamEvent::Text {
                text: result.output.clone(),
            });
            stream.emit(CognitiveStreamEvent::TurnDone {
                result: Ok(result.output.clone()),
            });
            events
                .emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: result.stop.clone(),
                })
                .await;
            return Ok(result);
        };

        self.inner.reset();
        let contract_message = self.configure_turn_contract(&request, services);
        let mut seed_messages = services.seed_messages(&request);
        if let Some(contract_message) = contract_message {
            seed_messages.push(contract_message);
        }
        if !seed_messages.is_empty() {
            self.inner.seed_messages(seed_messages);
        }
        self.inner.set_goal(request.input.clone());
        if let Ok(view) = services.dasein_view(request.process_id).await {
            let text = view.text.map(|text| bounded_dasein_context(&text));
            self.inner
                .set_dasein_context_provider(Box::new(move || text.clone()));
        }
        stream.emit(CognitiveStreamEvent::GoalSet {
            goal: request.input,
            sub_goals: vec![],
        });

        let tool_defs = services.tool_definitions();
        let process_id = request.process_id;
        let clock = self.clock.clone();
        let recording_llm = ProjectionRecordingLlm {
            inner: llm,
            services,
            operation_id: request.operation_id,
            pending: Arc::new(Mutex::new(Vec::new())),
        };
        let llm = DynLlmRef(&recording_llm);
        let sink = CognitiveStreamAdapter(stream);
        let run = self.inner.run_streaming(
            &llm,
            &tool_defs,
            |call_id, name, input| {
                let clock = clock.clone();
                let call = CapabilityCall {
                    operation_id: request.operation_id,
                    process_id,
                    name: name.to_string(),
                    input: input.clone(),
                    call_id: call_id.to_string(),
                    deadline: None,
                };
                async move {
                    let result = invoke_with_terminal_receipt(services, call, clock.as_ref()).await;
                    (result.output, result.is_error)
                }
            },
            || services.drain_interjections(),
            &sink,
        );
        let (output, metrics) = tokio::select! {
            _ = self.cancellation.cancelled() => {
                recording_llm.drain().await;
                events.emit(TurnEvent::Finished {
                    operation_id: request.operation_id,
                    stop: TurnStop::Cancelled,
                }).await;
                return Err(CognitError::cancelled());
            }
            result = run => match result {
                Ok(result) => result,
                Err(error) => {
                    recording_llm.drain().await;
                    events.emit(TurnEvent::Finished {
                        operation_id: request.operation_id,
                        stop: TurnStop::Failed,
                    }).await;
                    return Err(CognitError::from_runtime(error));
                }
            }
        };
        recording_llm.drain().await;
        let result = TurnResult {
            output,
            stop: metrics.stop.clone(),
            metrics: FabricTurnMetrics {
                tool_calls_made: metrics.tool_calls_made,
                tool_errors: metrics.tool_errors,
                provider_retries: metrics.provider_retries,
                elapsed_ms: metrics.elapsed_ms,
                iterations: metrics.iterations,
                completed_normally: metrics.completed_normally,
            },
        };
        events
            .emit(TurnEvent::Finished {
                operation_id: request.operation_id,
                stop: result.stop.clone(),
            })
            .await;
        Ok(result)
    }
}

fn bounded_error(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

fn bounded_dasein_context(content: &str) -> String {
    // Dasein state is durable host-projected context, not the current user
    // message. Scrub before bounding so prior outcomes cannot re-expose a
    // secret to a later turn or session.
    let governed = fabric::types::data_governance::scrub_for_projection(
        content,
        fabric::types::data_governance::ContentTrust::ExternalUntrusted,
    );
    let content = governed.content;
    if content.len() <= MAX_DASEIN_CONTEXT_BYTES {
        return content;
    }
    let mut boundary = MAX_DASEIN_CONTEXT_BYTES;
    while boundary > 0 && !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!(
        "{}\n... [existential context truncated from {} bytes]",
        &content[..boundary],
        content.len()
    )
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use fabric::{RecallRequest, RecallSet};
    use std::sync::Mutex as StdMutex;

    struct ReceiptServices {
        result: fabric::CapabilityResult,
        receipts: StdMutex<Vec<CapabilityTerminalReceipt>>,
    }

    #[async_trait]
    impl TurnServices for ReceiptServices {
        async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
            Ok(RecallSet::default())
        }
        async fn dasein_view(
            &self,
            _process: fabric::ProcessId,
        ) -> anyhow::Result<fabric::DaseinView> {
            Ok(fabric::DaseinView::default())
        }
        async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
            Ok(fabric::AgoraView::default())
        }
        async fn invoke(&self, _call: CapabilityCall) -> fabric::CapabilityResult {
            self.result.clone()
        }
        async fn record_capability_receipt(&self, receipt: CapabilityTerminalReceipt) {
            self.receipts.lock().unwrap().push(receipt);
        }
    }

    #[test]
    fn dasein_context_is_bounded_before_repeated_injection() {
        let input = "状态".repeat(MAX_DASEIN_CONTEXT_BYTES);
        let bounded = bounded_dasein_context(&input);

        assert!(bounded.len() <= MAX_DASEIN_CONTEXT_BYTES + 80);
        assert!(bounded.contains("existential context truncated"));
    }

    #[test]
    fn dasein_context_rescrubs_secret_bearing_durable_state() {
        let bounded = bounded_dasein_context(
            "prior outcome sk-daseinSecret123; API 密钥 daseinLabelSecret123",
        );

        assert!(!bounded.contains("sk-daseinSecret123"));
        assert!(!bounded.contains("daseinLabelSecret123"));
        assert_eq!(bounded.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn running_managed_command_does_not_create_terminal_receipt() {
        let result = fabric::CapabilityResult {
            call_id: "call".into(),
            output: serde_json::json!({
                "session_id": "session",
                "terminal": null,
                "truncated": false
            })
            .to_string(),
            is_error: false,
            usage: fabric::UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        };
        assert!(terminal_receipt_details("exec_command", &result).is_none());
    }

    #[test]
    fn agent_requirement_names_the_runtime_override_field() {
        let contract = render_turn_contract(
            None,
            &[fabric::TurnRequirement::InvokeAgentRuntime {
                runtime_id: "pi-rpc".into(),
            }],
        );
        assert!(contract.contains("`agent_spawn`"));
        assert!(contract.contains("`runtime` field set exactly to `pi-rpc`"));
        assert!(contract.contains("runtime ID is not a profile name"));
        assert!(contract.contains("`agent_wait`"));
    }

    #[test]
    fn terminal_validation_projects_exact_status_and_output_reference() {
        let result = fabric::CapabilityResult {
            call_id: "call".into(),
            output: serde_json::json!({
                "session_id": "session",
                "terminal": {"status": "exited", "exit_code": 0},
                "output_artifact_ref": "artifact://sha256/validation",
                "truncated": true
            })
            .to_string(),
            is_error: false,
            usage: fabric::UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        };
        let details = terminal_receipt_details("validation_run", &result).unwrap();
        assert_eq!(details.status, Some(CapabilityTerminalStatus::Succeeded));
        assert_eq!(details.exit_code, Some(0));
        assert_eq!(
            details.output_ref.as_deref(),
            Some("artifact://sha256/validation")
        );
        assert!(details.truncated);
    }

    #[tokio::test]
    async fn terminal_invocation_is_forwarded_to_receipt_port_once() {
        let services = ReceiptServices {
            result: fabric::CapabilityResult {
                call_id: "call".into(),
                output: "observed".into(),
                is_error: false,
                usage: fabric::UsageReport::default(),
                audit_id: None,
                patch_delta: None,
            },
            receipts: StdMutex::new(Vec::new()),
        };
        let call = CapabilityCall {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: "file_read".into(),
            input: serde_json::Value::Null,
            call_id: "call".into(),
            deadline: None,
        };
        let clock = kernel::chronos::TestClock::default();

        let _ = invoke_with_terminal_receipt(&services, call, &clock).await;

        let receipts = services.receipts.lock().unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].proves_success());
        assert_eq!(receipts[0].capability, "file_read");
    }
}
