use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fabric::Clock;
use fabric::Timer;
use tracing::warn;

use super::approval::{ApprovalDecision, ApprovalGate, ApprovalRequest, AutoDenyGate};
use super::audit::{AuditLogger, AuditRecord};
use super::escape_detector::{EscapePolicy, ShellEscalationDetector};
use super::loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
use super::output_guardrail::OutputGuardrail;
use super::policy::{PolicyEngine, PolicyVerdict};
use super::risk_classifier::RiskClassifier;
use crate::sandbox::executor::create_default_executor;
use crate::sandbox::{SandboxConfig, SandboxExecutor, SandboxPreference};
use crate::security::strategy::{resolve_strategy, ToolExecutionStrategy};
use crate::security::structured_sandbox::StructuredToolSandbox;
use fabric::execpolicy::{Decision as ExecDecision, Policy as ExecPolicy};
use fabric::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use fabric::{
    resolve_profile, PermissionBehavior, PermissionContext, ProfileName, ProfileResolveError,
    SandboxProfiles,
};

static SANDBOX_FS_VIOLATION_TOTAL: AtomicU64 = AtomicU64::new(0);
static SANDBOX_GLOB_OVERFLOW_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SandboxMetricSnapshot {
    pub sandbox_fs_violation_total: u64,
    pub sandbox_glob_overflow_total: u64,
}

pub fn sandbox_metrics() -> SandboxMetricSnapshot {
    SandboxMetricSnapshot {
        sandbox_fs_violation_total: SANDBOX_FS_VIOLATION_TOTAL.load(Ordering::Relaxed),
        sandbox_glob_overflow_total: SANDBOX_GLOB_OVERFLOW_TOTAL.load(Ordering::Relaxed),
    }
}

#[derive(Debug)]
pub enum ToolError {
    PolicyDenied { reason: String },
    LoopBlocked { reason: String },
    EscalateToHuman { reason: String },
    InterruptTurn { reason: String },
    MaxRetriesExceeded,
    ExecutionFailed(String),
    OutputRejected(String),
    AuditFailed(String),
    StructuredSandboxUnavailable { tool: String },
    StructuredSandboxUnsupported { tool: String },
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PolicyDenied { reason } => write!(f, "Policy denied: {}", reason),
            Self::LoopBlocked { reason } => write!(f, "Loop blocked: {}", reason),
            Self::EscalateToHuman { reason } => write!(f, "Escalate to human: {}", reason),
            Self::InterruptTurn { reason } => write!(f, "Turn interrupted: {}", reason),
            Self::MaxRetriesExceeded => write!(f, "Max retries exceeded"),
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {}", msg),
            Self::OutputRejected(msg) => write!(f, "Output rejected: {}", msg),
            Self::AuditFailed(msg) => write!(f, "Audit persistence failed: {}", msg),
            Self::StructuredSandboxUnavailable { tool } => write!(
                f,
                "Structured sandbox transport unavailable for '{tool}' (fail-closed)"
            ),
            Self::StructuredSandboxUnsupported { tool } => write!(
                f,
                "Structured sandbox transport does not support '{tool}' (fail-closed)"
            ),
        }
    }
}

pub struct GuardedToolExecution {
    pub result: std::result::Result<ToolResult, ToolError>,
    pub audit_id: fabric::AuditEventId,
}

impl std::error::Error for ToolError {}

pub struct ToolRunnerWithGuard {
    sandbox: SandboxExecutor,
    loop_detector: LoopDetector,
    policy_engine: PolicyEngine,
    output_guardrail: OutputGuardrail,
    audit_logger: AuditLogger,
    risk_classifier: RiskClassifier,
    /// Approval gate consulted before executing tools that require approval.
    /// Defaults to AutoDenyGate (conservative: preserves prior "deny L2+" behavior).
    approval_gate: Arc<dyn ApprovalGate>,
    /// Principal/thread/tool grants approved for the rest of one thread.
    session_approvals: std::collections::HashSet<fabric::ThreadGrantKey>,
    /// Permission context for mode/rule-based pre-approval.
    permission_ctx: PermissionContext,
    /// Independent execpolicy engine. When set, takes precedence over the inline PolicyEngine.
    exec_policy: Option<ExecPolicy>,
    /// Injected clock for deterministic time in tests.
    clock: Arc<dyn Clock>,
    /// S1 sandbox profiles from trusted daemon config. None = no profile layer
    /// (flag off or not configured); legacy behavior preserved.
    sandbox_profiles: Option<SandboxProfiles>,
    /// Canonical event spine used for S1 profile and violation observability.
    event_bus: Option<Arc<fabric::CanonicalEventBus>>,
    /// Isolated transport for structured mutations. Required when profile
    /// routing resolves such a tool to `Sandboxed`.
    structured_sandbox: Option<Arc<dyn StructuredToolSandbox>>,
}

impl ToolRunnerWithGuard {
    pub fn new(sandbox: SandboxExecutor, audit_logger: AuditLogger, clock: Arc<dyn Clock>) -> Self {
        Self {
            sandbox,
            loop_detector: LoopDetector::new(LoopDetectorConfig::default()),
            output_guardrail: OutputGuardrail::with_defaults(),
            policy_engine: PolicyEngine::with_defaults(),
            audit_logger,
            risk_classifier: RiskClassifier::with_defaults(),
            approval_gate: Arc::new(AutoDenyGate),
            session_approvals: std::collections::HashSet::new(),
            permission_ctx: PermissionContext::default(),
            exec_policy: None,
            clock,
            sandbox_profiles: None,
            event_bus: None,
            structured_sandbox: None,
        }
    }

    /// Create with default sandbox (Auto preference).
    pub fn with_default_sandbox(audit_logger: AuditLogger, clock: Arc<dyn Clock>) -> Self {
        use crate::sandbox::SandboxPreference;
        Self::new(
            create_default_executor(SandboxPreference::Auto, clock.clone()),
            audit_logger,
            clock,
        )
    }

    /// Create with an explicit sandbox preference.
    pub fn with_sandbox_preference(
        audit_logger: AuditLogger,
        preference: SandboxPreference,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::new(
            create_default_executor(preference, clock.clone()),
            audit_logger,
            clock,
        )
    }

    /// Set the approval gate used for actions that require approval.
    pub fn with_approval_gate(mut self, gate: Arc<dyn ApprovalGate>) -> Self {
        self.approval_gate = gate;
        self
    }

    /// Set the permission context for mode/rule-based pre-approval.
    pub fn with_permission_context(mut self, ctx: PermissionContext) -> Self {
        self.permission_ctx = ctx;
        self
    }

    /// Inject sandbox profiles for S1 profile-layer enforcement.
    /// When set (and `grok_hardening.sandbox_profiles` is on in the executive
    /// layer), the default profile is resolved before every `bash_exec` sandbox
    /// invocation and the resulting policy is carried in `SandboxConfig.policy`.
    /// `None` (the default) means no profile layer — byte-identical legacy.
    pub fn with_sandbox_profiles(mut self, profiles: SandboxProfiles) -> Self {
        self.sandbox_profiles = Some(profiles);
        self
    }

    pub fn with_event_bus(mut self, event_bus: Arc<fabric::CanonicalEventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    pub fn with_structured_sandbox(mut self, sandbox: Arc<dyn StructuredToolSandbox>) -> Self {
        self.structured_sandbox = Some(sandbox);
        self
    }

    async fn publish_sandbox_event(&self, schema: &'static str, payload: serde_json::Value) {
        let Some(event_bus) = &self.event_bus else {
            return;
        };
        if let Err(error) = event_bus
            .publish_event(fabric::SchemaId(schema.into()), "corpus.sandbox", payload)
            .await
        {
            tracing::warn!(schema, error = %error, "failed to publish sandbox event");
        }
    }

    /// Set the independent execpolicy engine. When set, this takes precedence
    /// over the inline PolicyEngine for policy decisions.
    pub fn with_policy(mut self, policy: ExecPolicy) -> Self {
        self.exec_policy = Some(policy);
        self
    }

    /// Check policy using execpolicy if available, otherwise fall back to inline PolicyEngine.
    fn check_policy(&self, tool_name: &str, input: &serde_json::Value) -> PolicyVerdict {
        if let Some(ref policy) = self.exec_policy {
            let cmd = Self::build_command_vec(tool_name, input);
            let eval = policy.check(&cmd, fabric::execpolicy::default_heuristics);
            match eval.decision {
                ExecDecision::Allow => PolicyVerdict::Allow,
                ExecDecision::Forbidden => PolicyVerdict::Deny {
                    reason: format!("Policy forbids: {}", eval.matched_rules.join(", ")),
                },
                ExecDecision::Prompt => PolicyVerdict::RequireApproval {
                    reason: format!(
                        "Policy requires approval: {}",
                        eval.matched_rules.join(", ")
                    ),
                },
            }
        } else {
            self.policy_engine.check(tool_name, input)
        }
    }

    /// Build a command vector from tool name and input for execpolicy evaluation.
    /// For bash_exec, appends the entire command string as a single token (no splitting)
    /// so that shell syntax (quotes, pipes, redirects) is preserved for policy matching.
    fn build_command_vec(tool_name: &str, input: &serde_json::Value) -> Vec<String> {
        let mut cmd = vec![tool_name.to_string()];
        if tool_name == "bash_exec" {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                cmd.push(command.to_string());
            }
        }
        cmd
    }

    pub fn on_new_turn(&mut self, turn_id: &str) {
        self.loop_detector.on_new_turn(turn_id);
    }

    pub fn end_turn(&mut self, turn_id: &str) {
        self.loop_detector.end_turn(turn_id);
    }

    /// Execute a tool with full security pipeline: policy -> loop detection ->
    /// sandbox execution -> output guardrail -> audit logging.
    pub async fn execute_tool(
        &mut self,
        tool: &dyn Tool,
        input: serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
    ) -> std::result::Result<ToolResult, ToolError> {
        self.execute_tool_report(tool, input, ctx, turn_id)
            .await
            .result
    }

    pub async fn execute_tool_report(
        &mut self,
        tool: &dyn Tool,
        input: serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
    ) -> GuardedToolExecution {
        let audit_id = fabric::AuditEventId::new();
        let result = self
            .execute_tool_inner(tool, input, ctx, turn_id, audit_id, None)
            .await;
        GuardedToolExecution { result, audit_id }
    }

    /// Streaming counterpart that preserves the same policy, loop, output and
    /// audit pipeline. Legacy/sandbox tools emit a terminal-only stream.
    pub async fn execute_tool_streaming_report(
        &mut self,
        tool: &dyn Tool,
        input: serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
        sink: &mut fabric::ToolEventSink,
    ) -> GuardedToolExecution {
        let audit_id = fabric::AuditEventId::new();
        let result = self
            .execute_tool_inner(tool, input, ctx, turn_id, audit_id, Some(sink))
            .await;
        GuardedToolExecution { result, audit_id }
    }

    async fn execute_tool_inner(
        &mut self,
        tool: &dyn Tool,
        input: serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
        audit_id: fabric::AuditEventId,
        mut sink: Option<&mut fabric::ToolEventSink>,
    ) -> std::result::Result<ToolResult, ToolError> {
        let tool_name = tool.name();
        let start = self.clock.mono_now();

        // 1. Policy check
        let policy_verdict = self.check_policy(tool_name, &input);
        match policy_verdict {
            PolicyVerdict::Deny { reason } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "denied",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::PolicyDenied { reason });
            }
            PolicyVerdict::RequireApproval { reason } => {
                if tool.permission_level() >= PermissionLevel::L2 {
                    let summary = input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|c| format!("{}: {}", tool_name, c))
                        .unwrap_or_else(|| format!("{}: {}", tool_name, input));

                    // Consult PermissionContext before the approval gate.
                    match self.permission_ctx.resolve(tool_name, &summary, true) {
                        PermissionBehavior::Allow => {
                            // Rule/mode pre-approves; skip approval gate.
                        }
                        PermissionBehavior::Deny => {
                            self.log_audit(
                                audit_id,
                                tool_name,
                                &input,
                                tool.permission_level(),
                                turn_id,
                                None,
                                &start,
                                "rule_denied",
                            )
                            .await
                            .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                            return Err(ToolError::PolicyDenied {
                                reason: format!("{}: denied by permission rule/mode", reason),
                            });
                        }
                        PermissionBehavior::Ask => {
                            // Fall through to existing approval-gate flow.
                            let Some(authority) = ctx.approval_authority.as_ref() else {
                                self.log_audit(
                                    audit_id,
                                    tool_name,
                                    &input,
                                    tool.permission_level(),
                                    turn_id,
                                    None,
                                    &start,
                                    "approval_authority_missing",
                                )
                                .await
                                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                                return Err(ToolError::PolicyDenied {
                                    reason: format!(
                                        "{}: authenticated approval authority is unavailable",
                                        reason
                                    ),
                                });
                            };
                            let grant_key = fabric::ThreadGrantKey {
                                owner: fabric::ApprovalOwner::new(
                                    authority.principal_id.clone(),
                                    authority.thread_id.clone(),
                                ),
                                tool: tool_name.to_owned(),
                            };
                            if self.session_approvals.contains(&grant_key) {
                                // Previously approved-for-session; allow.
                            } else {
                                let req = ApprovalRequest {
                                    owner: grant_key.owner.clone(),
                                    connection_id: authority.connection_id.clone(),
                                    turn_id: authority.turn_id,
                                    call_id: authority.call_id.clone(),
                                    workspace: authority.workspace.clone(),
                                    tool: tool_name.to_string(),
                                    action_summary: summary,
                                    risk_level: format!("{:?}", tool.permission_level()),
                                    detail: Some(input.to_string()),
                                };
                                match self.approval_gate.request(&req).await {
                                    ApprovalDecision::Approve => {}
                                    ApprovalDecision::ApproveForSession => {
                                        self.session_approvals.insert(grant_key);
                                    }
                                    ApprovalDecision::Deny => {
                                        self.log_audit(
                                            audit_id,
                                            tool_name,
                                            &input,
                                            tool.permission_level(),
                                            turn_id,
                                            None,
                                            &start,
                                            "approval_denied",
                                        )
                                        .await
                                        .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                                        return Err(ToolError::PolicyDenied {
                                            reason: format!("{}: denied by approval gate", reason),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            PolicyVerdict::Allow => {}
        }

        // 2. Loop detector pre-check
        let loop_verdict = self.loop_detector.pre_check(tool_name, &input, turn_id);
        match &loop_verdict {
            LoopVerdict::Allow => {}
            LoopVerdict::Warn { reason } => {
                warn!(tool = tool_name, reason = %reason, "Loop detector warning");
            }
            LoopVerdict::Block { reason, suggestion } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "loop_blocked",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::LoopBlocked {
                    reason: format!("{}. {}", reason, suggestion),
                });
            }
            LoopVerdict::Escalate { reason } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "escalated",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::EscalateToHuman {
                    reason: reason.clone(),
                });
            }
            LoopVerdict::InterruptTurn { reason, .. } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "interrupted",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::InterruptTurn {
                    reason: reason.clone(),
                });
            }
        }

        // 3. Determine the execution strategy for this tool. The profile layer
        // is the D1 feature flag boundary: when absent, preserve the legacy
        // contract exactly (only bash_exec is routed through SandboxExecutor).
        let strategy = if self.sandbox_profiles.is_none() {
            if tool_name == "bash_exec" {
                ToolExecutionStrategy::Sandboxed
            } else {
                ToolExecutionStrategy::InProcess
            }
        } else {
            resolve_strategy(tool_name, tool.permission_level())
        };

        let result = match strategy {
            ToolExecutionStrategy::Sandboxed | ToolExecutionStrategy::ExecServerRequired => {
                // Command tools use SandboxExecutor; structured tools use the
                // injected filesystem-capable transport below.
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

                let workspace = ctx
                    .effective_workspace_policy()
                    .map_err(|reason| ToolError::PolicyDenied { reason })?;
                let trusted_working_dir = workspace.cwd().to_string_lossy().to_string();

                // S1 T13: resolve the default sandbox profile when profiles are
                // configured.
                let policy = if let Some(profiles) = self.sandbox_profiles.as_ref() {
                    let name: ProfileName = profiles
                        .default_profile
                        .as_str()
                        .parse()
                        .unwrap_or(ProfileName::Workspace);
                    match resolve_profile(&name, &workspace, profiles) {
                        Ok(mut p) => {
                            let mut expansion_roots = vec![workspace.cwd().to_path_buf()];
                            expansion_roots.extend(workspace.writable_roots().iter().cloned());
                            expansion_roots.sort();
                            expansion_roots.dedup();
                            match fabric::expand_deny_globs(&p.deny_globs, &expansion_roots) {
                                Ok(expanded) => {
                                    for path in expanded {
                                        if !p.deny_exact.contains(&path) {
                                            p.deny_exact.push(path);
                                        }
                                    }
                                }
                                Err(e) => {
                                    SANDBOX_FS_VIOLATION_TOTAL.fetch_add(1, Ordering::Relaxed);
                                    SANDBOX_GLOB_OVERFLOW_TOTAL.fetch_add(1, Ordering::Relaxed);
                                    self.publish_sandbox_event(
                                        fabric::SchemaId::EVENT_SANDBOX_VIOLATION_V1,
                                        serde_json::json!({
                                            "event": "sandbox.violation",
                                            "target": name.to_string(),
                                            "operation": "expand_deny_globs",
                                            "principal": ctx.approval_authority.as_ref().map(|a| a.principal_id.0.as_str()),
                                            "agent": ctx.agent.as_ref().map(|a| a.parent_agent_id.0.to_string()),
                                            "reason": e.to_string(),
                                        }),
                                    )
                                    .await;
                                    return Err(ToolError::PolicyDenied {
                                        reason: format!(
                                            "sandbox profile '{name}' deny globs could not be expanded (fail-closed): {e}"
                                        ),
                                    });
                                }
                            }
                            tracing::debug!(
                                profile = %p.name,
                                restrict_network = p.restrict_network,
                                deny_exact = p.deny_exact.len(),
                                deny_globs = p.deny_globs.len(),
                                "resolved sandbox profile"
                            );
                            Some(p)
                        }
                        Err(e) => {
                            SANDBOX_FS_VIOLATION_TOTAL.fetch_add(1, Ordering::Relaxed);
                            if matches!(&e, ProfileResolveError::GlobOverflow) {
                                SANDBOX_GLOB_OVERFLOW_TOTAL.fetch_add(1, Ordering::Relaxed);
                            }
                            self.publish_sandbox_event(
                                fabric::SchemaId::EVENT_SANDBOX_VIOLATION_V1,
                                serde_json::json!({
                                    "event": "sandbox.violation",
                                    "target": name.to_string(),
                                    "operation": "resolve_profile",
                                    "principal": ctx.approval_authority.as_ref().map(|a| a.principal_id.0.as_str()),
                                    "agent": ctx.agent.as_ref().map(|a| a.parent_agent_id.0.to_string()),
                                    "reason": e.to_string(),
                                }),
                            )
                            .await;
                            return Err(ToolError::PolicyDenied {
                                reason: format!(
                                    "sandbox profile '{name}' could not be resolved (fail-closed): {e}"
                                ),
                            });
                        }
                    }
                } else {
                    None
                };

                let sandbox_config = SandboxConfig {
                    workspace,
                    environment: std::collections::BTreeMap::from([
                        ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
                        ("GIT_CONFIG_KEY_0".to_string(), "safe.directory".to_string()),
                        ("GIT_CONFIG_VALUE_0".to_string(), trusted_working_dir),
                    ]),
                    policy,
                };

                if let Some(policy) = &sandbox_config.policy {
                    self.publish_sandbox_event(
                        fabric::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1,
                        serde_json::json!({
                            "event": "sandbox.profile.applied",
                            "profile": policy.name,
                            "read_only": policy.read_only_roots,
                            "read_write": policy.read_write_roots,
                            "deny_exact": policy.deny_exact,
                            "deny_globs": policy.deny_globs,
                            "restrict_network": policy.restrict_network,
                            "principal": ctx.approval_authority.as_ref().map(|a| a.principal_id.0.as_str()),
                            "agent": ctx.agent.as_ref().map(|a| a.parent_agent_id.0.to_string()),
                        }),
                    )
                    .await;
                }

                // `bash_exec` is the only command-shaped built-in in the
                // strategy table. Every other sandboxed tool carries a
                // structured JSON contract and must cross the isolated port;
                // treating a missing `command` field as `""` would otherwise
                // turn a mutation/build request into a successful no-op.
                if tool_name != "bash_exec" {
                    let transport = self.structured_sandbox.as_ref().ok_or_else(|| {
                        ToolError::StructuredSandboxUnavailable {
                            tool: tool_name.to_owned(),
                        }
                    })?;
                    if !transport.supports_tool(tool_name) {
                        return Err(ToolError::StructuredSandboxUnsupported {
                            tool: tool_name.to_owned(),
                        });
                    }
                    transport
                        .execute(tool_name, input.clone(), ctx, &sandbox_config)
                        .await
                        .map_err(|error| {
                            ToolError::ExecutionFailed(format!(
                                "structured sandbox execution failed for '{tool_name}': {error}"
                            ))
                        })?
                } else {
                    // D1-T11: the trusted sandbox-profile gate also enables shell
                    // escape detection. Legacy execution (profiles absent) remains
                    // byte-for-byte unchanged, while hardened execution blocks
                    // high-severity bypass patterns before any process is started.
                    if sandbox_config.policy.is_some() {
                        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
                        match detector.evaluate(cmd) {
                            Ok(detections) => {
                                for detection in detections {
                                    warn!(
                                        command = cmd,
                                        pattern = detection.pattern,
                                        severity = ?detection.severity,
                                        "shell escalation pattern observed"
                                    );
                                }
                            }
                            Err(detection) => {
                                SANDBOX_FS_VIOLATION_TOTAL.fetch_add(1, Ordering::Relaxed);
                                self.publish_sandbox_event(
                                fabric::SchemaId::EVENT_SANDBOX_VIOLATION_V1,
                                serde_json::json!({
                                    "event": "sandbox.violation",
                                    "target": cmd,
                                    "operation": "shell_escape_detection",
                                    "pattern": detection.pattern,
                                    "severity": format!("{:?}", detection.severity),
                                    "principal": ctx.approval_authority.as_ref().map(|a| a.principal_id.0.as_str()),
                                    "agent": ctx.agent.as_ref().map(|a| a.parent_agent_id.0.to_string()),
                                    "reason": detection.description,
                                }),
                            )
                            .await;
                                return Err(ToolError::PolicyDenied {
                                    reason: format!(
                                        "shell escalation pattern '{}' blocked: {}",
                                        detection.pattern, detection.description
                                    ),
                                });
                            }
                        }
                    }

                    let sandbox_result = match sink.as_deref_mut() {
                        Some(sink) => {
                            self.sandbox
                                .run_streaming(cmd, &sandbox_config, Duration::from_secs(30), sink)
                                .await
                        }
                        None => {
                            self.sandbox
                                .run(cmd, &sandbox_config, Duration::from_secs(30))
                                .await
                        }
                    };
                    // Only backend/enforcement errors become violation events. A
                    // command can forge stderr text, so ordinary non-zero child
                    // exits must never be promoted to authoritative violations.
                    let violation = sandbox_result.as_ref().err().map(ToString::to_string);
                    if sandbox_config.policy.is_some() {
                        if let Some(reason) = violation {
                            SANDBOX_FS_VIOLATION_TOTAL.fetch_add(1, Ordering::Relaxed);
                            self.publish_sandbox_event(
                            fabric::SchemaId::EVENT_SANDBOX_VIOLATION_V1,
                            serde_json::json!({
                                "event": "sandbox.violation",
                                "target": cmd,
                                "operation": "execute",
                                "principal": ctx.approval_authority.as_ref().map(|a| a.principal_id.0.as_str()),
                                "agent": ctx.agent.as_ref().map(|a| a.parent_agent_id.0.to_string()),
                                "reason": reason,
                            }),
                        )
                        .await;
                        }
                    }
                    match sandbox_result {
                        Ok(sandbox_result) => ToolResult {
                            content: format!(
                                "{}\n{}",
                                sandbox_result.stdout, sandbox_result.stderr
                            )
                            .trim()
                            .to_string(),
                            is_error: sandbox_result.exit_code != 0,
                            metadata: ToolResultMeta {
                                execution_time_ms: sandbox_result.elapsed_ms,
                                truncated: false,
                            },
                        },
                        Err(e) => ToolResult {
                            content: format!("Sandbox execution failed: {}", e),
                            is_error: true,
                            metadata: ToolResultMeta {
                                execution_time_ms: 0,
                                truncated: false,
                            },
                        },
                    }
                }
            }
            ToolExecutionStrategy::InProcess | ToolExecutionStrategy::NetworkProxied { .. } => {
                // Structured tools execute through their implementation with a
                // bounded timeout. Path-mutating tools enforce canonical workspace
                // confinement in their own implementation.
                // NetworkProxied is Phase 2+; in Phase 1 it falls through to InProcess.
                const TOOL_TIMEOUT_SECS: u64 = 60;
                let execution = async {
                    if let Some(sink) = sink.as_deref_mut() {
                        tool.execute_streaming(input.clone(), ctx, sink).await;
                        match sink.terminal_result().cloned() {
                            Some(Ok(result)) => result,
                            Some(Err(error)) => ToolResult {
                                content: error.to_string(),
                                is_error: true,
                                metadata: ToolResultMeta::default(),
                            },
                            None => ToolResult {
                                content: fabric::ToolExecutionError::NoTerminal.to_string(),
                                is_error: true,
                                metadata: ToolResultMeta::default(),
                            },
                        }
                    } else {
                        tool.execute(input.clone(), ctx).await
                    }
                };
                match aletheon_kernel::chronos::SystemTimer
                    .timeout(Duration::from_secs(TOOL_TIMEOUT_SECS), execution)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => ToolResult {
                        content: format!(
                            "Tool '{}' timed out after {}s",
                            tool_name, TOOL_TIMEOUT_SECS
                        ),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: TOOL_TIMEOUT_SECS * 1000,
                            truncated: false,
                        },
                    },
                }
            }
        };

        // 4. Validate captured output without re-running a side effect.
        let final_result = result;
        let output_rejection = self
            .output_guardrail
            .validate(&final_result)
            .await
            .err()
            .map(|error| format!("{error:?}"));

        // 5. Loop detector post-check
        self.loop_detector
            .post_check(tool_name, &input, &final_result, turn_id);

        // 6. Audit log
        let verdict_str = format!("{:?}", loop_verdict);
        self.log_audit(
            audit_id,
            tool_name,
            &input,
            tool.permission_level(),
            turn_id,
            Some(&final_result),
            &start,
            &verdict_str,
        )
        .await
        .map_err(|e| ToolError::AuditFailed(e.to_string()))?;

        if let Some(reason) = output_rejection {
            return Err(ToolError::OutputRejected(reason));
        }

        if let Some(sink) = sink {
            if !sink.terminal_sent() {
                sink.terminal(Ok(final_result.clone())).await;
            }
        }

        Ok(final_result)
    }

    /// Legacy entry point — delegates to execute_tool, returning ToolResult
    /// directly for backward compatibility.
    pub async fn run(
        &mut self,
        tool: &dyn Tool,
        args: serde_json::Value,
        ctx: &ToolContext,
        turn_id: &str,
    ) -> ToolResult {
        match self.execute_tool(tool, args, ctx, turn_id).await {
            Ok(result) => result,
            Err(e) => ToolResult {
                content: e.to_string(),
                is_error: true,
                metadata: Default::default(),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn log_audit(
        &self,
        audit_id: fabric::AuditEventId,
        tool_name: &str,
        input: &serde_json::Value,
        level: PermissionLevel,
        turn_id: &str,
        result: Option<&ToolResult>,
        start: &fabric::MonoTime,
        verdict: &str,
    ) -> anyhow::Result<()> {
        let category = self.risk_classifier.classify(tool_name);
        let record = AuditRecord {
            audit_id,
            timestamp: self.clock.wall_now(),
            session_id: String::new(), // Will be filled by caller or context
            turn_id: turn_id.to_string(),
            tool_name: tool_name.to_string(),
            args: input.clone(),
            permission_level: level,
            risk_category: category,
            loop_verdict: verdict.to_string(),
            result_summary: result.map(|r| r.content.chars().take(200).collect()),
            is_error: result.map(|r| r.is_error).unwrap_or(false),
            sandbox_backend: None,
            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
        };
        self.audit_logger.log(record).await
    }

    pub fn metrics(&self) -> &super::loop_detector::LoopDetectorMetrics {
        &self.loop_detector.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::super::approval::{AutoApproveGate, AutoDenyGate};
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use async_trait::async_trait;
    use fabric::execpolicy::{Decision as ExecDecision, PrefixRule as ExecPrefixRule};
    use fabric::tool::{
        ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult,
        ToolResultMeta,
    };
    use fabric::{PermissionContext, PermissionMode};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A dummy L2 tool used to exercise the approval gate path.
    /// Named "bash_exec" so the policy engine's `rm -rf *` rule triggers RequireApproval.
    struct DummyL2Tool;

    struct StructuredL1Tool {
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    struct StreamingReadTool;

    #[async_trait]
    impl Tool for StreamingReadTool {
        fn name(&self) -> &str {
            "streaming_read"
        }

        fn description(&self) -> &str {
            "streaming read operation"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::L0
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            panic!("legacy execute path must not run when streaming is enabled")
        }

        async fn execute_streaming(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
            sink: &mut fabric::ToolEventSink,
        ) {
            assert!(sink.progress(fabric::ToolProgress::Text("phase-1".into())));
            assert!(sink.progress(fabric::ToolProgress::Structured(
                serde_json::json!({"pct": 100})
            )));
            sink.terminal(Ok(ToolResult {
                content: "streamed-result".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            }))
            .await;
        }

        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(self.clone())
        }
    }

    #[async_trait]
    impl Tool for StructuredL1Tool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "structured read operation"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::L1
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ToolResult {
                content: "wrote artifact".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            }
        }
        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(Self {
                name: self.name,
                calls: Arc::clone(&self.calls),
            })
        }
    }

    struct MockStructuredSandbox {
        calls: Arc<AtomicUsize>,
        expected_tool: &'static str,
    }

    #[async_trait]
    impl StructuredToolSandbox for MockStructuredSandbox {
        async fn execute(
            &self,
            tool_name: &str,
            input: serde_json::Value,
            _context: &ToolContext,
            sandbox: &SandboxConfig,
        ) -> Result<ToolResult, String> {
            assert_eq!(tool_name, self.expected_tool);
            assert_eq!(input["content"], "written");
            let policy = sandbox.policy.as_ref().expect("resolved policy required");
            assert_eq!(policy.name, "workspace");
            assert!(policy
                .read_write_roots
                .iter()
                .any(|root| root == sandbox.workspace.cwd()));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult {
                content: "sandbox wrote artifact".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            })
        }
    }

    #[async_trait]
    impl Tool for DummyL2Tool {
        fn name(&self) -> &str {
            "bash_exec"
        }
        fn description(&self) -> &str {
            "Dummy L2 tool for testing"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::L2
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult {
                content: "ok".into(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            }
        }
        fn boxed_clone(&self) -> Box<dyn Tool> {
            Box::new(DummyL2Tool)
        }
        fn exposure(&self) -> ToolExposure {
            ToolExposure::Direct
        }
        fn concurrency_class(&self) -> ConcurrencyClass {
            ConcurrencyClass::SideEffect
        }
    }

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    fn make_runner(gate: Arc<dyn ApprovalGate>) -> ToolRunnerWithGuard {
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        ToolRunnerWithGuard::with_sandbox_preference(
            audit_logger,
            SandboxPreference::Forbid,
            test_clock(),
        )
        .with_approval_gate(gate)
    }

    fn make_input_rm() -> serde_json::Value {
        serde_json::json!({ "command": "rm -rf /tmp/test" })
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            approval_authority: Some(fabric::ToolApprovalAuthority {
                principal_id: fabric::PrincipalId("test".into()),
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId("test-session".into()),
                turn_id: fabric::TurnId::new(),
                call_id: "test-call".into(),
                workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![])
                    .unwrap(),
            }),
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test-session".into(),
            clock: test_clock(),
        }
    }

    #[tokio::test]
    async fn structured_mutations_preserve_legacy_execution_when_profiles_are_disabled() {
        for name in ["file_write", "apply_patch"] {
            let calls = Arc::new(AtomicUsize::new(0));
            let tool = StructuredL1Tool {
                name,
                calls: Arc::clone(&calls),
            };
            let mut runner = make_runner(Arc::new(AutoApproveGate));

            let result = runner
                .execute_tool(
                    &tool,
                    serde_json::json!({"path": "artifact.txt"}),
                    &make_ctx(),
                    "structured-turn",
                )
                .await
                .unwrap();

            assert_eq!(result.content, "wrote artifact");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn structured_mutations_fail_closed_without_transport_when_profiles_are_enabled() {
        for name in ["file_write", "apply_patch"] {
            let calls = Arc::new(AtomicUsize::new(0));
            let tool = StructuredL1Tool {
                name,
                calls: Arc::clone(&calls),
            };
            let mut runner = make_runner(Arc::new(AutoApproveGate))
                .with_sandbox_profiles(fabric::SandboxProfiles::default());

            let error = runner
                .execute_tool(
                    &tool,
                    serde_json::json!({"path": "artifact.txt", "content": "written"}),
                    &make_ctx(),
                    "profile-structured-turn",
                )
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                ToolError::StructuredSandboxUnavailable { ref tool } if tool == name
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn unsupported_structured_sandbox_strategy_fails_closed_without_empty_command() {
        let in_process_calls = Arc::new(AtomicUsize::new(0));
        let transport_calls = Arc::new(AtomicUsize::new(0));
        let tool = StructuredL1Tool {
            name: "ebpf_compile",
            calls: Arc::clone(&in_process_calls),
        };
        let mut runner = make_runner(Arc::new(AutoApproveGate))
            .with_sandbox_profiles(fabric::SandboxProfiles::default())
            .with_structured_sandbox(Arc::new(MockStructuredSandbox {
                calls: Arc::clone(&transport_calls),
                expected_tool: "file_write",
            }));

        let error = runner
            .execute_tool(
                &tool,
                serde_json::json!({"source_path": "program.c"}),
                &make_ctx(),
                "unsupported-structured-transport-turn",
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ToolError::StructuredSandboxUnsupported { ref tool } if tool == "ebpf_compile"
        ));
        assert_eq!(in_process_calls.load(Ordering::SeqCst), 0);
        assert_eq!(transport_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unknown_configured_default_profile_fails_closed() {
        let in_process_calls = Arc::new(AtomicUsize::new(0));
        let tool = StructuredL1Tool {
            name: "file_write",
            calls: Arc::clone(&in_process_calls),
        };
        let profiles = fabric::SandboxProfiles {
            default_profile: "missing-trusted-profile".into(),
            ..fabric::SandboxProfiles::default()
        };
        let mut runner = make_runner(Arc::new(AutoApproveGate)).with_sandbox_profiles(profiles);

        let error = runner
            .execute_tool(
                &tool,
                serde_json::json!({"path": "artifact.txt", "content": "written"}),
                &make_ctx(),
                "unknown-profile-turn",
            )
            .await
            .unwrap_err();

        assert!(matches!(error, ToolError::PolicyDenied { ref reason }
            if reason.contains("missing-trusted-profile") && reason.contains("fail-closed")));
        assert_eq!(in_process_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn structured_mutations_use_sandbox_transport_when_profiles_are_enabled() {
        for name in ["file_write", "apply_patch"] {
            let in_process_calls = Arc::new(AtomicUsize::new(0));
            let transport_calls = Arc::new(AtomicUsize::new(0));
            let tool = StructuredL1Tool {
                name,
                calls: Arc::clone(&in_process_calls),
            };
            let mut runner = make_runner(Arc::new(AutoApproveGate))
                .with_sandbox_profiles(fabric::SandboxProfiles::default())
                .with_structured_sandbox(Arc::new(MockStructuredSandbox {
                    calls: Arc::clone(&transport_calls),
                    expected_tool: name,
                }));

            let result = runner
                .execute_tool(
                    &tool,
                    serde_json::json!({"path": "artifact.txt", "content": "written"}),
                    &make_ctx(),
                    "profile-structured-transport-turn",
                )
                .await
                .unwrap();

            assert_eq!(result.content, "sandbox wrote artifact");
            assert_eq!(in_process_calls.load(Ordering::SeqCst), 0);
            assert_eq!(transport_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn guarded_streaming_executes_override_once_and_preserves_terminal() {
        let mut runner = make_runner(Arc::new(AutoApproveGate));
        let (mut sink, mut rx) = fabric::tool_event_channel();

        let report = runner
            .execute_tool_streaming_report(
                &StreamingReadTool,
                serde_json::json!({}),
                &make_ctx(),
                "streaming-turn",
                &mut sink,
            )
            .await;

        let result = report.result.expect("guarded streaming result");
        assert_eq!(result.content, "streamed-result");
        assert!(matches!(
            rx.recv().await,
            Some(fabric::ToolExecutionEvent::Progress(
                fabric::ToolProgress::Text(_)
            ))
        ));
        assert!(matches!(
            rx.recv().await,
            Some(fabric::ToolExecutionEvent::Progress(
                fabric::ToolProgress::Structured(_)
            ))
        ));
        assert!(matches!(
            rx.recv().await,
            Some(fabric::ToolExecutionEvent::Terminal(Ok(_)))
        ));
    }

    #[tokio::test]
    async fn bash_sandbox_streams_multiple_lines_and_one_terminal() {
        let mut runner = make_runner(Arc::new(AutoApproveGate));
        let (mut sink, mut rx) = fabric::tool_event_channel();
        let report = runner
            .execute_tool_streaming_report(
                &DummyL2Tool,
                serde_json::json!({
                    "command": "printf 'alpha\\n'; sleep 0.05; printf 'beta\\n'"
                }),
                &make_ctx(),
                "bash-streaming-turn",
                &mut sink,
            )
            .await;

        assert!(report.result.is_ok());
        let mut progress = Vec::new();
        let mut terminals = 0;
        while let Ok(event) = rx.try_recv() {
            match event {
                fabric::ToolExecutionEvent::Progress(fabric::ToolProgress::Text(line)) => {
                    progress.push(line);
                }
                fabric::ToolExecutionEvent::Terminal(_) => terminals += 1,
                _ => {}
            }
        }
        assert_eq!(progress, ["alpha", "beta"]);
        assert_eq!(terminals, 1);
    }

    #[tokio::test]
    async fn sandbox_profile_applied_event_carries_policy_and_principal() {
        let bus = Arc::new(fabric::CanonicalEventBus::new(8));
        let mut events = bus.subscribe_channel(fabric::SchemaId(
            fabric::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
        ));
        let mut runner = make_runner(Arc::new(AutoApproveGate))
            .with_sandbox_profiles(fabric::SandboxProfiles::default())
            .with_event_bus(bus);

        runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf ok"}),
                &make_ctx(),
                "sandbox-event-turn",
            )
            .await
            .expect("sandboxed execution");

        let event = events.recv().await.expect("profile applied event");
        assert_eq!(event.payload["event"], "sandbox.profile.applied");
        assert_eq!(event.payload["profile"], "workspace");
        assert_eq!(event.payload["principal"], "test");
        assert!(event.payload.get("read_write").is_some());
        assert!(event.payload.get("deny_exact").is_some());
        assert!(event.payload.get("restrict_network").is_some());
    }

    #[tokio::test]
    async fn sandbox_profile_resolution_violation_is_attributed_and_fails_closed() {
        let bus = Arc::new(fabric::CanonicalEventBus::new(8));
        let mut events = bus.subscribe_channel(fabric::SchemaId(
            fabric::SchemaId::EVENT_SANDBOX_VIOLATION_V1.into(),
        ));
        let profiles = fabric::SandboxProfiles {
            default_profile: "missing-custom".into(),
            ..Default::default()
        };
        let mut runner = make_runner(Arc::new(AutoApproveGate))
            .with_sandbox_profiles(profiles)
            .with_event_bus(bus);
        let before = sandbox_metrics();

        let result = runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf must-not-run"}),
                &make_ctx(),
                "sandbox-violation-turn",
            )
            .await;

        assert!(matches!(result, Err(ToolError::PolicyDenied { .. })));
        let event = events.recv().await.expect("sandbox violation event");
        assert_eq!(event.payload["event"], "sandbox.violation");
        assert_eq!(event.payload["target"], "missing-custom");
        assert_eq!(event.payload["operation"], "resolve_profile");
        assert_eq!(event.payload["principal"], "test");
        assert!(sandbox_metrics().sandbox_fs_violation_total > before.sandbox_fs_violation_total);
    }

    #[tokio::test]
    async fn configured_deny_globs_are_expanded_before_backend_execution() {
        let workspace = tempfile::tempdir().unwrap();
        let denied = workspace.path().join(".env");
        std::fs::write(&denied, "secret").unwrap();
        let mut ctx = make_ctx();
        ctx.working_dir = workspace.path().to_path_buf();
        ctx.approval_authority.as_mut().unwrap().workspace =
            fabric::WorkspacePolicy::from_resolved_roots(
                workspace.path().to_path_buf(),
                Vec::new(),
            )
            .unwrap();
        let profiles = fabric::SandboxProfiles {
            default_profile: "guarded".into(),
            profiles: std::collections::BTreeMap::from([(
                "guarded".into(),
                fabric::SandboxProfileConfig {
                    extends: Some("workspace".into()),
                    restrict_network: None,
                    read_only: Vec::new(),
                    read_write: Vec::new(),
                    deny: vec!["**/.env".into()],
                },
            )]),
        };
        let bus = Arc::new(fabric::CanonicalEventBus::new(8));
        let mut events = bus.subscribe_channel(fabric::SchemaId(
            fabric::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
        ));
        let mut runner = make_runner(Arc::new(AutoApproveGate))
            .with_sandbox_profiles(profiles)
            .with_event_bus(bus);

        runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf ok"}),
                &ctx,
                "glob-expansion-turn",
            )
            .await
            .expect("sandboxed execution");

        let event = events.recv().await.expect("profile applied event");
        assert!(event.payload["deny_exact"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == denied.to_str().unwrap()));
    }

    #[tokio::test]
    async fn deny_glob_overflow_is_counted_and_fails_closed() {
        let profiles = fabric::SandboxProfiles {
            default_profile: "overflow".into(),
            profiles: std::collections::BTreeMap::from([(
                "overflow".into(),
                fabric::SandboxProfileConfig {
                    extends: Some("workspace".into()),
                    restrict_network: None,
                    read_only: Vec::new(),
                    read_write: Vec::new(),
                    deny: (0..=fabric::DENY_GLOB_MAX_ENTRIES)
                        .map(|index| format!("**/secret-{index}*"))
                        .collect(),
                },
            )]),
        };
        let before = sandbox_metrics();
        let mut runner = make_runner(Arc::new(AutoApproveGate)).with_sandbox_profiles(profiles);

        let result = runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf must-not-run"}),
                &make_ctx(),
                "glob-overflow-turn",
            )
            .await;

        assert!(matches!(result, Err(ToolError::PolicyDenied { .. })));
        assert!(sandbox_metrics().sandbox_glob_overflow_total > before.sandbox_glob_overflow_total);
    }

    #[tokio::test]
    async fn child_agent_context_cannot_widen_the_daemon_bound_profile() {
        let bus = Arc::new(fabric::CanonicalEventBus::new(8));
        let mut events = bus.subscribe_channel(fabric::SchemaId(
            fabric::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
        ));
        let mut runner = make_runner(Arc::new(AutoApproveGate))
            .with_sandbox_profiles(fabric::SandboxProfiles::default())
            .with_event_bus(bus);
        let parent_ctx = make_ctx();
        let mut child_ctx = make_ctx();
        child_ctx.agent = Some(fabric::AgentToolContext {
            caller_root_agent_id: fabric::AgentId::new(),
            parent_agent_id: fabric::AgentId::new(),
            parent_process_id: fabric::ProcessId::new(),
        });

        runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf parent"}),
                &parent_ctx,
                "parent-profile-turn",
            )
            .await
            .unwrap();
        runner
            .execute_tool(
                &DummyL2Tool,
                serde_json::json!({"command": "printf child"}),
                &child_ctx,
                "child-profile-turn",
            )
            .await
            .unwrap();

        let parent = events.recv().await.unwrap();
        let child = events.recv().await.unwrap();
        assert_eq!(parent.payload["profile"], child.payload["profile"]);
        assert_eq!(child.payload["profile"], "workspace");
        assert!(child.payload["agent"].is_string());
    }

    #[tokio::test]
    async fn l2_denied_by_gate_is_blocked() {
        let mut runner = make_runner(Arc::new(AutoDenyGate));
        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(result.is_err(), "AutoDenyGate should deny L2 tool");
        match result.unwrap_err() {
            ToolError::PolicyDenied { reason } => {
                assert!(
                    reason.contains("denied by approval gate"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("Expected PolicyDenied, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn l2_approved_by_gate_runs() {
        let mut runner = make_runner(Arc::new(AutoApproveGate));
        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
            "AutoApproveGate should pass policy before output validation: {result:?}"
        );
    }

    #[tokio::test]
    async fn bypass_all_approves_l2() {
        // BypassAll mode should allow L2 tool without any approval gate prompt.
        let ctx = PermissionContext {
            mode: PermissionMode::BypassAll,
            ..Default::default()
        };
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
            audit_logger,
            SandboxPreference::Forbid,
            test_clock(),
        )
        .with_approval_gate(Arc::new(AutoDenyGate))
        .with_permission_context(ctx);
        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
            "BypassAll should pass policy even if captured output is rejected: {result:?}"
        );
    }

    #[tokio::test]
    async fn plan_mode_denies_dangerous() {
        // Plan mode should deny L2 (dangerous) tool, audit as "rule_denied".
        let ctx = PermissionContext {
            mode: PermissionMode::Plan,
            ..Default::default()
        };
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
            .with_approval_gate(Arc::new(AutoApproveGate))
            .with_permission_context(ctx);
        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(result.is_err(), "Plan mode should deny L2 tool");
        match result.unwrap_err() {
            ToolError::PolicyDenied { reason } => {
                assert!(
                    reason.contains("denied by permission rule/mode"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("Expected PolicyDenied, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn runner_uses_execpolicy_for_deny() {
        // Build an execpolicy that forbids "bash_exec" entirely.
        let mut policy = ExecPolicy::new();
        policy.add_rule(ExecPrefixRule::new("bash_exec", ExecDecision::Forbidden));

        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
            .with_policy(policy);

        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(result.is_err(), "execpolicy should deny bash_exec");
        match result.unwrap_err() {
            ToolError::PolicyDenied { reason } => {
                assert!(reason.contains("Policy forbids"), "reason: {}", reason);
            }
            other => panic!("Expected PolicyDenied, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn runner_execpolicy_prompt_triggers_approval() {
        // Build an execpolicy that prompts for "bash_exec".
        let mut policy = ExecPolicy::new();
        policy.add_rule(ExecPrefixRule::new("bash_exec", ExecDecision::Prompt));

        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
            .with_approval_gate(Arc::new(AutoApproveGate))
            .with_policy(policy);

        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
            "approval should pass before output validation: {result:?}"
        );
    }

    #[tokio::test]
    async fn runner_no_execpolicy_falls_back_to_policy_engine() {
        // Without with_policy(), the inline PolicyEngine is used.
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
            .with_approval_gate(Arc::new(AutoApproveGate));

        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
            "inline policy should pass before output validation: {result:?}"
        );
    }

    #[test]
    fn build_command_vec_extracts_bash_command() {
        let input = serde_json::json!({ "command": "rm -rf /tmp/test" });
        let cmd = ToolRunnerWithGuard::build_command_vec("bash_exec", &input);
        // Command string is kept as a single token to preserve shell syntax.
        assert_eq!(cmd, vec!["bash_exec", "rm -rf /tmp/test"]);
    }

    #[test]
    fn build_command_vec_non_bash_tool() {
        let input = serde_json::json!({ "path": "/tmp/file.txt" });
        let cmd = ToolRunnerWithGuard::build_command_vec("file_read", &input);
        assert_eq!(cmd, vec!["file_read"]);
    }
}
