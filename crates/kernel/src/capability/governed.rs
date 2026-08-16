//! Kernel-owned authorization boundary for cognitive capability calls.
//!
//! Cognit supplies only a [`CapabilityCall`].  This module attaches trusted
//! application authority and cancellation before delegating to Kernel's
//! admit/execute/settle pipeline.

use crate::AdmissionController;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use ::contracts::types::admission::RiskLevel;
use ::contracts::{
    BroadcastEpoch, CapabilityAuthority, CapabilityCall, CapabilityResult, CapabilityScope,
    ConsciousArbitrationMode, ContentId, FieldDecisionKind, FieldDecisionReason, InvocationControl,
    PrincipalId, ProcessId, SalienceVector, SandboxRequirement, UsageReport, WorkspaceAttribution,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use crate::capability::{CapabilityInvoker, DefaultCapabilityInvoker, ToolExecutor};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub use crate::capability::canonical_capability_invoker;

pub use crate::capability::{canonical_permit_issuer, GovernedPermitIssuer};

/// Trusted execution context attached by host composition, never by model input.
#[derive(Clone)]
pub struct CapabilityExecutionContext {
    pub agent: Option<::contracts::AgentToolContext>,
    pub process_id: ::contracts::ProcessId,
    pub operation_id: ::contracts::OperationId,
    pub principal: PrincipalId,
    pub connection_id: ::contracts::ConnectionId,
    pub thread_id: ::contracts::ThreadId,
    pub turn_id: ::contracts::TurnId,
    /// Host-authenticated per-turn target. Model input cannot author this value.
    pub execution_target: ::contracts::ExecutionTargetSelection,
    pub workspace: ::contracts::WorkspacePolicy,
    pub permission_mode: ::contracts::permission::HostPermissionMode,
    pub session_id: String,
    pub working_dir: PathBuf,
    pub sandbox: SandboxRequirement,
    pub cancel: CancellationToken,
    pub turn_count: usize,
    pub repo_hooks_trusted: bool,
    pub action_loop: Option<Arc<dyn GovernedActionLoop>>,
    /// G2 additive progress path. Both fields are required to activate it.
    pub streaming_tools: bool,
    pub turn_event_sender: Option<::contracts::ipc::TurnEventSender>,
}

#[async_trait]
pub trait GovernedActionLoopResolver: Send + Sync {
    async fn resolve(
        &self,
        space: ::contracts::AgoraSpaceId,
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
    pub operation_id: ::contracts::OperationId,
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
    stream_events: Option<::contracts::ipc::TurnEventSender>,
    notification_observer: Option<runtime::tool_stream_bridge::ToolNotificationObserver>,
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

    pub fn with_tool_stream(mut self, sender: ::contracts::ipc::TurnEventSender) -> Self {
        self.stream_events = Some(sender);
        self
    }

    pub fn with_notification_observer(
        mut self,
        observer: runtime::tool_stream_bridge::ToolNotificationObserver,
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
                    served_from_cache: false,
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
                                    served_from_cache: false,
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
                            served_from_cache: false,
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
                                served_from_cache: false,
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
                            served_from_cache: false,
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
                        served_from_cache: false,
                    };
                }
            }
        } else {
            None
        };
        let observed_call = call.clone();
        let request = ::contracts::CapabilityRequest {
            call,
            authority: authorized.authority,
            control: authorized.control,
        };
        let result = if let Some(turn_events) = &self.stream_events {
            let tool_name = request.call.name.clone();
            let call_id = request.call.call_id.clone();
            let cancel = request.control.cancel.clone();
            let (mut sink, event_rx) = ::contracts::tool_event_channel_for_call(call_id.clone());
            let inner = self.inner.clone();
            let invoke = async move { inner.invoke_streaming(request, &mut sink).await };
            let bridge = runtime::tool_stream_bridge::bridge_bound_tool_stream_observed(
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
                    served_from_cache: false,
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
        sender: ::contracts::ipc::TurnEventSender,
        notification_observer: Option<runtime::tool_stream_bridge::ToolNotificationObserver>,
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
    agent: Option<::contracts::AgentToolContext>,
    risk_by_tool: HashMap<String, RiskLevel>,
    principal: PrincipalId,
    connection_id: ::contracts::ConnectionId,
    thread_id: ::contracts::ThreadId,
    turn_id: ::contracts::TurnId,
    workspace: ::contracts::WorkspacePolicy,
    session_id: String,
    working_dir: PathBuf,
    sandbox: SandboxRequirement,
    cancel: CancellationToken,
    turn_event_sender: Option<::contracts::ipc::TurnEventSender>,
    permission_mode: ::contracts::permission::HostPermissionMode,
    execution_target: ::contracts::ExecutionTargetSelection,
}

impl RegistryAuthorityProvider {
    pub fn new(
        risk_by_tool: HashMap<String, RiskLevel>,
        principal: PrincipalId,
        connection_id: ::contracts::ConnectionId,
        thread_id: ::contracts::ThreadId,
        turn_id: ::contracts::TurnId,
        workspace: ::contracts::WorkspacePolicy,
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
            permission_mode: ::contracts::permission::HostPermissionMode::Safe,
            execution_target: ::contracts::ExecutionTargetSelection::default(),
        }
    }

    pub fn with_agent_context(mut self, agent: Option<::contracts::AgentToolContext>) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_turn_event_sender(
        mut self,
        sender: Option<::contracts::ipc::TurnEventSender>,
    ) -> Self {
        self.turn_event_sender = sender;
        self
    }

    pub fn with_permission_mode(
        mut self,
        permission_mode: ::contracts::permission::HostPermissionMode,
    ) -> Self {
        self.permission_mode = permission_mode;
        self
    }

    pub fn with_execution_target(
        mut self,
        execution_target: ::contracts::ExecutionTargetSelection,
    ) -> Self {
        self.execution_target = execution_target;
        self
    }
}

fn validate_robot_tool_target(
    call: &CapabilityCall,
    selection: &::contracts::ExecutionTargetSelection,
) -> Result<()> {
    #[derive(Deserialize)]
    struct RobotToolTargetInput {
        #[serde(default)]
        device: Option<String>,
    }

    if !call.name.starts_with("robot_") {
        return Ok(());
    }
    selection.validate().map_err(anyhow::Error::msg)?;
    let ::contracts::ExecutionTarget::Robot { device_id, .. } = &selection.target else {
        if matches!(
            call.name.as_str(),
            "robot_execute_skill" | "robot_cancel" | "robot_safe_stop"
        ) {
            anyhow::bail!(
                "execution_target_required: robot actuation capability '{}' requires an explicit Robot target",
                call.name
            );
        }
        return Ok(());
    };
    let input = serde_json::from_value::<RobotToolTargetInput>(call.input.clone())
        .map_err(|error| anyhow!("invalid robot capability target input: {error}"))?;
    if let Some(requested_device) = input.device {
        anyhow::ensure!(
            requested_device == device_id.0,
            "execution_target_mismatch: robot capability '{}' requested device '{}' but the turn is bound to '{}'",
            call.name,
            requested_device,
            device_id.0
        );
    }
    Ok(())
}

fn requested_scope_for_call(
    call: &CapabilityCall,
    workspace: &::contracts::WorkspacePolicy,
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
        #[derive(Deserialize)]
        struct FileReadScopeInput {
            path: Option<String>,
            #[serde(default)]
            paths: Vec<String>,
        }

        let parsed: FileReadScopeInput = serde_json::from_value(call.input.clone())
            .map_err(|error| anyhow!("invalid file_read scope input: {error}"))?;
        let mut requested_paths = parsed.paths;
        if let Some(path) = parsed.path {
            requested_paths.insert(0, path);
        }
        if requested_paths.is_empty() {
            return Err(anyhow!("file_read requires path or paths before admission"));
        }
        for requested in requested_paths {
            let candidate = if std::path::Path::new(&requested).is_absolute() {
                PathBuf::from(&requested)
            } else {
                workspace.cwd().join(&requested)
            };
            let has_parent_traversal = std::path::Path::new(&requested)
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir));
            let inside_workspace = !has_parent_traversal
                && workspace
                    .writable_roots()
                    .iter()
                    .any(|root| candidate.starts_with(root));
            if inside_workspace {
                // Workspace roots are already present in the requested scope.
                // Do not require a speculative read target to exist merely to
                // authorize it; Corpus will return a per-file not-found result.
                continue;
            }
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
    }

    Ok(CapabilityScope {
        allowed_paths,
        ..CapabilityScope::default()
    })
}

#[async_trait]
impl TurnAuthorityProvider for RegistryAuthorityProvider {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        validate_robot_tool_target(call, &self.execution_target)?;
        let risk = *self
            .risk_by_tool
            .get(&call.name)
            .ok_or_else(|| anyhow!("unknown tool '{}'", call.name))?;
        let requested_scope = requested_scope_for_call(call, &self.workspace)?;
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                agent: self.agent.clone(),
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
                permission_mode: self.permission_mode,
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
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: name.into(),
            input: serde_json::json!({"path": path}),
            call_id: "scope-test".into(),
            deadline: None,
        }
    }

    #[test]
    fn file_write_requests_workspace_roots_not_empty_any_scope() {
        let root = tempfile::tempdir().unwrap();
        let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
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
        let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
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

    #[test]
    fn general_target_cannot_authorize_robot_actuation_from_prompt_text() {
        let execute = CapabilityCall {
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: "robot_execute_skill".into(),
            input: serde_json::json!({"device":"robot-1","skill":"move"}),
            call_id: "robot-general".into(),
            deadline: None,
        };
        let error =
            validate_robot_tool_target(&execute, &::contracts::ExecutionTargetSelection::default())
                .unwrap_err();
        assert!(error.to_string().contains("execution_target_required"));

        let mut observe = execute.clone();
        observe.name = "robot_observe".into();
        assert!(validate_robot_tool_target(
            &observe,
            &::contracts::ExecutionTargetSelection::default()
        )
        .is_ok());
    }

    #[test]
    fn explicit_robot_target_enforces_exact_device_binding() {
        let target = ::contracts::ExecutionTargetSelection::robot(
            "robot-1",
            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            ::contracts::ExecutionTargetSource::TrustedClient,
        )
        .unwrap();
        let mut call = CapabilityCall {
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: "robot_execute_skill".into(),
            input: serde_json::json!({"device":"robot-1","skill":"move"}),
            call_id: "robot-explicit".into(),
            deadline: None,
        };
        assert!(validate_robot_tool_target(&call, &target).is_ok());
        call.input["device"] = serde_json::json!("robot-2");
        let error = validate_robot_tool_target(&call, &target).unwrap_err();
        assert!(error.to_string().contains("execution_target_mismatch"));
    }

    #[test]
    fn file_read_batch_requests_each_exact_external_path() {
        let workspace_root = tempfile::tempdir().unwrap();
        let external_root = tempfile::tempdir().unwrap();
        let first = external_root.path().join("first.txt");
        let second = external_root.path().join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
            workspace_root.path().canonicalize().unwrap(),
            vec![],
        )
        .unwrap();
        let call = CapabilityCall {
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: "file_read".into(),
            input: serde_json::json!({"paths": [first, second]}),
            call_id: "batch-scope-test".into(),
            deadline: None,
        };

        let scope = requested_scope_for_call(&call, &workspace).unwrap();
        assert!(scope
            .allowed_paths
            .contains(&first.canonicalize().unwrap().to_string_lossy().into_owned()));
        assert!(scope.allowed_paths.contains(
            &second
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ));
        assert!(!scope
            .allowed_paths
            .contains(&external_root.path().to_string_lossy().into_owned()));
    }

    #[test]
    fn file_read_admits_missing_workspace_path_for_tool_level_reporting() {
        let workspace_root = tempfile::tempdir().unwrap();
        let root = workspace_root.path().canonicalize().unwrap();
        let workspace =
            ::contracts::WorkspacePolicy::from_resolved_roots(root.clone(), vec![]).unwrap();
        let missing = root.join("package.json");
        let call = CapabilityCall {
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: "file_read".into(),
            input: serde_json::json!({"paths": ["README.md", missing]}),
            call_id: "missing-workspace-path".into(),
            deadline: None,
        };

        let scope = requested_scope_for_call(&call, &workspace).unwrap();
        assert_eq!(scope.allowed_paths, vec![root.to_string_lossy()]);
    }

    #[test]
    fn file_read_does_not_admit_parent_traversal_as_workspace_relative() {
        let workspace_root = tempfile::tempdir().unwrap();
        let root = workspace_root.path().canonicalize().unwrap();
        let workspace =
            ::contracts::WorkspacePolicy::from_resolved_roots(root.clone(), vec![]).unwrap();
        let call = CapabilityCall {
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            name: "file_read".into(),
            input: serde_json::json!({"path": "../missing.txt"}),
            call_id: "parent-traversal".into(),
            deadline: None,
        };

        assert!(requested_scope_for_call(&call, &workspace).is_err());
    }
}
