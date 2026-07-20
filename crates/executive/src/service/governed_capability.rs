//! Executive-owned authorization boundary for cognitive capability calls.
//!
//! Cognit supplies only a [`CapabilityCall`].  This module attaches trusted
//! application authority and cancellation before delegating to Kernel's
//! admit/execute/settle pipeline.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, BroadcastEpoch, CapabilityAuthority, CapabilityCall, CapabilityInvoker,
    CapabilityResult, CapabilityScope, ConsciousArbitrationMode, ContentId, FieldDecisionKind,
    FieldDecisionReason, InvocationControl, PrincipalId, ProcessId, SalienceVector,
    SandboxRequirement, UsageReport, WorkspaceAttribution,
};
use kernel::capability::{DefaultCapabilityInvoker, ToolExecutor};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

/// Trusted execution context attached by Executive, never by model input.
#[derive(Clone)]
pub struct CapabilityExecutionContext {
    pub agent: Option<fabric::AgentToolContext>,
    pub process_id: fabric::ProcessId,
    pub operation_id: fabric::OperationId,
    pub principal: PrincipalId,
    pub connection_id: fabric::ConnectionId,
    pub thread_id: fabric::ThreadId,
    pub turn_id: fabric::TurnId,
    pub workspace: fabric::WorkspacePolicy,
    pub session_id: String,
    pub working_dir: PathBuf,
    pub sandbox: SandboxRequirement,
    pub cancel: CancellationToken,
    pub turn_count: usize,
    pub repo_hooks_trusted: bool,
    pub action_loop: Option<Arc<dyn GovernedActionLoop>>,
    /// G2 additive progress path. Both fields are required to activate it.
    pub streaming_tools: bool,
    pub turn_event_sender: Option<fabric::ipc::TurnEventSender>,
}

#[async_trait]
pub trait GovernedActionLoopResolver: Send + Sync {
    async fn resolve(
        &self,
        space: fabric::AgoraSpaceId,
        source: ProcessId,
        root: ProcessId,
    ) -> Result<Arc<dyn GovernedActionLoop>>;
}

/// Canonical application capability entry point used outside the turn pipeline.
///
/// An existing lifecycle context is supplied by native sub-agents. Callers such
/// as MCP and durable goal workers pass `None`; the Executive implementation
/// creates and cleans up a transient Kernel Process/Operation around the call.
#[async_trait]
pub trait CapabilityService: Send + Sync {
    async fn invoke(
        &self,
        context: Option<CapabilityExecutionContext>,
        call: CapabilityCall,
        cancel: CancellationToken,
    ) -> CapabilityResult;
}

/// Trusted application result of authorizing a model-originated call.
pub struct AuthorizedInvocation {
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedActionContext {
    pub candidate_id: ContentId,
    pub broadcast_epoch: BroadcastEpoch,
    pub operation_id: fabric::OperationId,
    pub source_process: ProcessId,
    pub attribution: WorkspaceAttribution,
}

/// Bounded pre-execution evidence for a conscious action decision.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionModulationSnapshot {
    pub decision: FieldDecisionKind,
    pub reason: FieldDecisionReason,
    pub broadcast_epoch: BroadcastEpoch,
    pub confidence: f32,
    pub salience: SalienceVector,
    pub metric_ref: String,
}

impl ActionModulationSnapshot {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "action modulation confidence must be finite and in [0,1]"
        );
        self.salience.validate()?;
        anyhow::ensure!(
            matches!(
                (self.decision, self.reason),
                (FieldDecisionKind::Proceed, FieldDecisionReason::Selected)
                    | (FieldDecisionKind::Reorder, FieldDecisionReason::Selected)
                    | (
                        FieldDecisionKind::WouldDefer | FieldDecisionKind::Defer,
                        FieldDecisionReason::Negated | FieldDecisionReason::LostCompetition
                    )
            ),
            "action modulation decision and reason are inconsistent"
        );
        anyhow::ensure!(
            !self.metric_ref.trim().is_empty() && self.metric_ref.len() <= 32 * 1024,
            "action modulation metric reference is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GovernedActionDecision {
    Proceed {
        selected: SelectedActionContext,
        modulation: Option<ActionModulationSnapshot>,
    },
    Defer {
        reason: FieldDecisionReason,
        retryable: bool,
        modulation: ActionModulationSnapshot,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedActionOutcomeReceipt {
    pub outcome_id: ContentId,
    pub permit_id: String,
    pub broadcast_epoch: BroadcastEpoch,
}

#[async_trait]
pub trait GovernedActionLoop: Send + Sync {
    fn arbitration_mode(&self) -> ConsciousArbitrationMode {
        ConsciousArbitrationMode::Observe
    }

    async fn select_action(&self, call: &CapabilityCall) -> Result<GovernedActionDecision>;

    async fn observe_modulation(
        &self,
        mode: ConsciousArbitrationMode,
        call: &CapabilityCall,
        modulation: &ActionModulationSnapshot,
    ) -> Result<()>;

    async fn observe_outcome(
        &self,
        selected: &SelectedActionContext,
        call: &CapabilityCall,
        result: &CapabilityResult,
    ) -> Result<SelectedActionOutcomeReceipt>;
}

#[async_trait]
pub trait TurnAuthorityProvider: Send + Sync {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation>;
}

/// The only capability surface exposed to a turn implementation.
#[async_trait]
pub trait TurnCapabilityInvoker: Send + Sync {
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult;
}

pub struct GovernedCapabilityInvoker {
    inner: Arc<dyn CapabilityInvoker>,
    authority: Arc<dyn TurnAuthorityProvider>,
    action_loop: Option<Arc<dyn GovernedActionLoop>>,
    arbitration_mode: ConsciousArbitrationMode,
    stream_events: Option<fabric::ipc::TurnEventSender>,
    notification_observer: Option<crate::service::tool_stream_bridge::ToolNotificationObserver>,
}

impl GovernedCapabilityInvoker {
    pub fn new(
        inner: Arc<dyn CapabilityInvoker>,
        authority: Arc<dyn TurnAuthorityProvider>,
    ) -> Self {
        Self {
            inner,
            authority,
            action_loop: None,
            arbitration_mode: ConsciousArbitrationMode::Observe,
            stream_events: None,
            notification_observer: None,
        }
    }

    pub fn with_action_loop(mut self, action_loop: Arc<dyn GovernedActionLoop>) -> Self {
        self.arbitration_mode = action_loop.arbitration_mode();
        self.action_loop = Some(action_loop);
        self
    }

    /// Override conscious arbitration for an explicitly configured runtime.
    pub fn with_arbitration_mode(mut self, mode: ConsciousArbitrationMode) -> Self {
        self.arbitration_mode = mode;
        self
    }

    pub fn with_tool_stream(mut self, sender: fabric::ipc::TurnEventSender) -> Self {
        self.stream_events = Some(sender);
        self
    }

    pub fn with_notification_observer(
        mut self,
        observer: crate::service::tool_stream_bridge::ToolNotificationObserver,
    ) -> Self {
        self.notification_observer = Some(observer);
        self
    }
}

#[derive(Serialize)]
struct ConsciousDeferredPayload {
    code: &'static str,
    retryable: bool,
    reason: FieldDecisionReason,
    epoch: u64,
}

#[async_trait]
impl TurnCapabilityInvoker for GovernedCapabilityInvoker {
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        let authorized = match self.authority.authorize(&call).await {
            Ok(authorized) => authorized,
            Err(error) => {
                return CapabilityResult {
                    call_id: call.call_id,
                    output: format!("capability authorization denied: {error}"),
                    is_error: true,
                    usage: UsageReport::default(),
                    audit_id: None,
                    patch_delta: None,
                };
            }
        };
        let selected = if let Some(action_loop) = &self.action_loop {
            match action_loop.select_action(&call).await {
                Ok(GovernedActionDecision::Proceed {
                    selected,
                    modulation,
                }) => {
                    if let Some(modulation) = modulation.as_ref() {
                        if let Err(error) = action_loop
                            .observe_modulation(self.arbitration_mode, &call, modulation)
                            .await
                        {
                            if self.arbitration_mode == ConsciousArbitrationMode::Enforce {
                                return CapabilityResult {
                                    call_id: call.call_id,
                                    output: format!(
                                        "capability action modulation observation failed: {error}"
                                    ),
                                    is_error: true,
                                    usage: UsageReport::default(),
                                    audit_id: None,
                                    patch_delta: None,
                                };
                            }
                            tracing::warn!(
                                error = %error,
                                "conscious action modulation observation failed in observe mode"
                            );
                        }
                    }
                    Some(selected)
                }
                Ok(GovernedActionDecision::Defer {
                    reason,
                    retryable,
                    mut modulation,
                }) => {
                    modulation.decision = match self.arbitration_mode {
                        ConsciousArbitrationMode::Observe => FieldDecisionKind::WouldDefer,
                        ConsciousArbitrationMode::Enforce => FieldDecisionKind::Defer,
                    };
                    modulation.reason = reason;
                    if let Err(error) = modulation.validate() {
                        return CapabilityResult {
                            call_id: call.call_id,
                            output: format!("capability action modulation is invalid: {error}"),
                            is_error: true,
                            usage: UsageReport::default(),
                            audit_id: None,
                            patch_delta: None,
                        };
                    }
                    if let Err(error) = action_loop
                        .observe_modulation(self.arbitration_mode, &call, &modulation)
                        .await
                    {
                        if self.arbitration_mode == ConsciousArbitrationMode::Enforce {
                            return CapabilityResult {
                                call_id: call.call_id,
                                output: format!(
                                    "capability action modulation observation failed: {error}"
                                ),
                                is_error: true,
                                usage: UsageReport::default(),
                                audit_id: None,
                                patch_delta: None,
                            };
                        }
                        tracing::warn!(
                            error = %error,
                            "conscious would-defer observation failed in observe mode"
                        );
                    }
                    if self.arbitration_mode == ConsciousArbitrationMode::Enforce {
                        let payload = ConsciousDeferredPayload {
                            code: "consciousness_deferred",
                            retryable,
                            reason,
                            epoch: modulation.broadcast_epoch.0,
                        };
                        let output = serde_json::to_string(&payload).unwrap_or_else(|_| {
                            r#"{"code":"consciousness_deferred","retryable":false,"reason":"serialization_error","epoch":0}"#.into()
                        });
                        return CapabilityResult {
                            call_id: call.call_id,
                            output,
                            is_error: true,
                            usage: UsageReport::default(),
                            audit_id: None,
                            patch_delta: None,
                        };
                    }
                    None
                }
                Err(error) => {
                    return CapabilityResult {
                        call_id: call.call_id,
                        output: format!("capability action was not selected: {error}"),
                        is_error: true,
                        usage: UsageReport::default(),
                        audit_id: None,
                        patch_delta: None,
                    };
                }
            }
        } else {
            None
        };
        let observed_call = call.clone();
        let request = fabric::CapabilityRequest {
            call,
            authority: authorized.authority,
            control: authorized.control,
        };
        let result = if let Some(turn_events) = &self.stream_events {
            let tool_name = request.call.name.clone();
            let call_id = request.call.call_id.clone();
            let cancel = request.control.cancel.clone();
            let (mut sink, event_rx) = fabric::tool_event_channel_for_call(call_id.clone());
            let inner = self.inner.clone();
            let invoke = async move { inner.invoke_streaming(request, &mut sink).await };
            let bridge = crate::service::tool_stream_bridge::bridge_bound_tool_stream_observed(
                event_rx,
                turn_events.clone(),
                tool_name,
                call_id,
                cancel,
                self.notification_observer.clone(),
            );
            let (mut result, outcome) = tokio::join!(invoke, bridge);
            if let Err(error) = outcome.terminal {
                if !result.is_error {
                    result.output = format!("streaming tool execution failed: {error}");
                    result.is_error = true;
                }
            }
            result
        } else {
            self.inner.invoke(request).await
        };
        if let (Some(action_loop), Some(selected)) = (&self.action_loop, selected.as_ref()) {
            if let Err(error) = action_loop
                .observe_outcome(selected, &observed_call, &result)
                .await
            {
                return CapabilityResult {
                    call_id: result.call_id,
                    output: format!(
                        "capability executed but governed outcome recurrence failed: {error}"
                    ),
                    is_error: true,
                    usage: result.usage,
                    audit_id: result.audit_id,
                    patch_delta: None,
                };
            }
        }
        result
    }
}

/// Per-runtime composition factory shared by daemon and exec composition roots.
pub struct CapabilityRuntimeFactory;

impl CapabilityRuntimeFactory {
    pub fn build(
        admission: Arc<dyn AdmissionController>,
        executor: Arc<dyn ToolExecutor>,
        authority: Arc<dyn TurnAuthorityProvider>,
    ) -> Arc<dyn TurnCapabilityInvoker> {
        let kernel: Arc<dyn CapabilityInvoker> =
            Arc::new(DefaultCapabilityInvoker::new(admission, executor));
        Arc::new(GovernedCapabilityInvoker::new(kernel, authority))
    }

    pub fn build_with_action_loop(
        admission: Arc<dyn AdmissionController>,
        executor: Arc<dyn ToolExecutor>,
        authority: Arc<dyn TurnAuthorityProvider>,
        action_loop: Arc<dyn GovernedActionLoop>,
    ) -> Arc<dyn TurnCapabilityInvoker> {
        let kernel: Arc<dyn CapabilityInvoker> =
            Arc::new(DefaultCapabilityInvoker::new(admission, executor));
        Arc::new(GovernedCapabilityInvoker::new(kernel, authority).with_action_loop(action_loop))
    }

    pub fn build_streaming(
        admission: Arc<dyn AdmissionController>,
        executor: Arc<dyn ToolExecutor>,
        authority: Arc<dyn TurnAuthorityProvider>,
        action_loop: Option<Arc<dyn GovernedActionLoop>>,
        sender: fabric::ipc::TurnEventSender,
        notification_observer: Option<crate::service::tool_stream_bridge::ToolNotificationObserver>,
    ) -> Arc<dyn TurnCapabilityInvoker> {
        let kernel: Arc<dyn CapabilityInvoker> =
            Arc::new(DefaultCapabilityInvoker::new(admission, executor));
        let mut governed =
            GovernedCapabilityInvoker::new(kernel, authority).with_tool_stream(sender);
        if let Some(observer) = notification_observer {
            governed = governed.with_notification_observer(observer);
        }
        if let Some(action_loop) = action_loop {
            governed = governed.with_action_loop(action_loop);
        }
        Arc::new(governed)
    }
}

/// Registry-backed policy adapter. Unknown tools are rejected before admission;
/// known tools derive risk from their declared permission level.
pub struct RegistryAuthorityProvider {
    agent: Option<fabric::AgentToolContext>,
    risk_by_tool: HashMap<String, RiskLevel>,
    principal: PrincipalId,
    connection_id: fabric::ConnectionId,
    thread_id: fabric::ThreadId,
    turn_id: fabric::TurnId,
    workspace: fabric::WorkspacePolicy,
    session_id: String,
    working_dir: PathBuf,
    sandbox: SandboxRequirement,
    cancel: CancellationToken,
    turn_event_sender: Option<fabric::ipc::TurnEventSender>,
}

impl RegistryAuthorityProvider {
    pub fn new(
        risk_by_tool: HashMap<String, RiskLevel>,
        principal: PrincipalId,
        connection_id: fabric::ConnectionId,
        thread_id: fabric::ThreadId,
        turn_id: fabric::TurnId,
        workspace: fabric::WorkspacePolicy,
        session_id: String,
        _working_dir: PathBuf,
        sandbox: SandboxRequirement,
        cancel: CancellationToken,
    ) -> Self {
        let working_dir = workspace.cwd().to_path_buf();
        Self {
            agent: None,
            risk_by_tool,
            principal,
            connection_id,
            thread_id,
            turn_id,
            workspace,
            session_id,
            working_dir,
            sandbox,
            cancel,
            turn_event_sender: None,
        }
    }

    pub fn with_agent_context(mut self, agent: Option<fabric::AgentToolContext>) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_turn_event_sender(mut self, sender: Option<fabric::ipc::TurnEventSender>) -> Self {
        self.turn_event_sender = sender;
        self
    }
}

fn requested_scope_for_call(
    call: &CapabilityCall,
    workspace: &fabric::WorkspacePolicy,
) -> Result<CapabilityScope> {
    if !matches!(
        call.name.as_str(),
        "file_read" | "file_write" | "apply_patch"
    ) {
        return Ok(CapabilityScope::default());
    }

    let mut allowed_paths = workspace
        .writable_roots()
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();

    // Read-only access outside the workspace is deliberately exact-path, not
    // parent-directory authority. Corpus applies the sensitive-path denylist
    // before projecting this grant into a Platform FilesystemScope.
    if call.name == "file_read" {
        let requested = call
            .input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("file_read requires a path before admission"))?;
        let candidate = if std::path::Path::new(requested).is_absolute() {
            PathBuf::from(requested)
        } else {
            workspace.cwd().join(requested)
        };
        let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
            anyhow!(
                "file_read path '{}' cannot be admitted: {error}",
                candidate.display()
            )
        })?;
        let canonical = canonical.to_string_lossy().to_string();
        if !allowed_paths.contains(&canonical) {
            allowed_paths.push(canonical);
        }
    }

    Ok(CapabilityScope {
        allowed_paths,
        ..CapabilityScope::default()
    })
}

#[async_trait]
impl TurnAuthorityProvider for RegistryAuthorityProvider {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        let risk = *self
            .risk_by_tool
            .get(&call.name)
            .ok_or_else(|| anyhow!("unknown tool '{}'", call.name))?;
        let requested_scope = requested_scope_for_call(call, &self.workspace)?;
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                agent: self.agent,
                principal: self.principal.clone(),
                action: call.name.clone(),
                requested_scope,
                risk,
                budget: None,
                lease: None,
                sandbox: self.sandbox,
                connection_id: self.connection_id.clone(),
                thread_id: self.thread_id.clone(),
                turn_id: self.turn_id,
                workspace: self.workspace.clone(),
                session_id: self.session_id.clone(),
                working_dir: self.working_dir.clone(),
            },
            control: InvocationControl {
                cancel: self.cancel.clone(),
                turn_event_sender: self.turn_event_sender.clone(),
            },
        })
    }
}

#[cfg(test)]
mod filesystem_scope_tests {
    use super::*;

    fn call(name: &str, path: &std::path::Path) -> CapabilityCall {
        CapabilityCall {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: name.into(),
            input: serde_json::json!({"path": path}),
            call_id: "scope-test".into(),
            deadline: None,
        }
    }

    #[test]
    fn file_write_requests_workspace_roots_not_empty_any_scope() {
        let root = tempfile::tempdir().unwrap();
        let workspace = fabric::WorkspacePolicy::from_resolved_roots(
            root.path().canonicalize().unwrap(),
            vec![],
        )
        .unwrap();
        let scope = requested_scope_for_call(
            &call("file_write", PathBuf::from("x").as_path()),
            &workspace,
        )
        .unwrap();
        assert_eq!(scope.allowed_paths, vec![root.path().to_string_lossy()]);
    }

    #[test]
    fn file_read_requests_one_exact_external_path() {
        let workspace_root = tempfile::tempdir().unwrap();
        let external_root = tempfile::tempdir().unwrap();
        let external = external_root.path().join("diagnostic.txt");
        std::fs::write(&external, "diagnostic").unwrap();
        let workspace = fabric::WorkspacePolicy::from_resolved_roots(
            workspace_root.path().canonicalize().unwrap(),
            vec![],
        )
        .unwrap();
        let scope = requested_scope_for_call(&call("file_read", &external), &workspace).unwrap();
        assert!(scope.allowed_paths.contains(
            &external
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ));
        assert!(!scope
            .allowed_paths
            .contains(&external_root.path().to_string_lossy().into_owned()));
    }
}
