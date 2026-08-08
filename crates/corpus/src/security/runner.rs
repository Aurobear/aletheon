use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fabric::Clock;
use fabric::Timer;
use sha2::{Digest, Sha256};
use tracing::warn;

use super::approval::{ApprovalDecision, ApprovalGate, ApprovalRequest, AutoDenyGate};
use super::audit::{AuditLogger, AuditRecord};
use super::command_effect::{classify_command, CommandEffect};
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

/// Build the deliberately small environment exposed to sandboxed commands.
///
/// Bubblewrap clears the daemon environment to avoid leaking credentials.  A
/// completely empty environment is not useful either: installed user daemons
/// must still expose ordinary host toolchains selected through `PATH`, and
/// rustup needs its read-only home directories to resolve the active compiler.
/// Keep this list about process/toolchain identity only; provider credentials,
/// agent secrets, wrapper hooks, and mutation-oriented build variables are not
/// forwarded.
pub(crate) fn sandbox_command_environment(
    trusted_working_dir: String,
    scratch: Option<&std::path::Path>,
) -> std::collections::BTreeMap<String, String> {
    sandbox_command_environment_with(trusted_working_dir, scratch, |key| std::env::var(key))
}

fn sandbox_command_environment_with(
    trusted_working_dir: String,
    scratch: Option<&std::path::Path>,
    mut read: impl FnMut(&str) -> Result<String, std::env::VarError>,
) -> std::collections::BTreeMap<String, String> {
    const PASSTHROUGH: &[&str] = &[
        "HOME",
        "PATH",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "LANG",
        "LC_ALL",
        "TZ",
        "TERM",
    ];

    let mut environment = std::collections::BTreeMap::new();
    for key in PASSTHROUGH {
        if let Ok(value) = read(key) {
            environment.insert((*key).to_owned(), value);
        }
    }
    if let Some(home) = environment.get("HOME").cloned() {
        environment
            .entry("CARGO_HOME".to_owned())
            .or_insert_with(|| format!("{home}/.cargo"));
        environment
            .entry("RUSTUP_HOME".to_owned())
            .or_insert_with(|| format!("{home}/.rustup"));
    }

    environment.extend([
        ("GIT_CONFIG_COUNT".to_owned(), "1".to_owned()),
        ("GIT_CONFIG_KEY_0".to_owned(), "safe.directory".to_owned()),
        ("GIT_CONFIG_VALUE_0".to_owned(), trusted_working_dir),
    ]);
    if let Some(scratch) = scratch {
        let scratch = scratch.to_string_lossy().into_owned();
        environment.extend([
            ("TMPDIR".to_owned(), scratch.clone()),
            ("TMP".to_owned(), scratch.clone()),
            ("TEMP".to_owned(), scratch),
        ]);
    }
    environment
}

mod cache_hit;

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
            Self::PolicyDenied { reason } => write!(f, "Policy denied: {reason}"),
            Self::LoopBlocked { reason } => write!(f, "Loop blocked: {reason}"),
            Self::EscalateToHuman { reason } => write!(f, "Escalate to human: {reason}"),
            Self::InterruptTurn { reason } => write!(f, "Turn interrupted: {reason}"),
            Self::MaxRetriesExceeded => write!(f, "Max retries exceeded"),
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {msg}"),
            Self::OutputRejected(msg) => write!(f, "Output rejected: {msg}"),
            Self::AuditFailed(msg) => write!(f, "Audit persistence failed: {msg}"),
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
    fn check_policy(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        unrestricted: bool,
    ) -> PolicyVerdict {
        if unrestricted {
            return PolicyVerdict::Allow;
        }
        if tool_name == "exec_command" {
            let command = input
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            match classify_command(command) {
                CommandEffect::Destructive => {
                    return PolicyVerdict::Deny {
                        reason: "destructive managed commands are forbidden".into(),
                    };
                }
                CommandEffect::SystemChange => {
                    return PolicyVerdict::Deny {
                        reason: "system package, service, and privilege changes are unavailable inside the production command sandbox; report this host boundary and stop without retrying or calling repo_inspect".into(),
                    };
                }
                CommandEffect::NetworkEgress | CommandEffect::ReadOnlyNetwork => {
                    return PolicyVerdict::RequireApproval {
                        reason: "managed command requests external network access".into(),
                    };
                }
                CommandEffect::ReadOnly | CommandEffect::WorkspaceMutation => {}
            }
        }
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
        sink.defer_terminal_delivery();
        let result = self
            .execute_tool_inner(tool, input, ctx, turn_id, audit_id, Some(sink))
            .await;
        sink.settle_deferred_execution(&result).await;
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
        let mut sandbox_backend: Option<String> = None;
        let unrestricted = ctx
            .approval_authority
            .as_ref()
            .map(|authority| authority.permission_mode.is_full())
            .unwrap_or(false);

        // 1. Policy check
        let policy_verdict = self.check_policy(tool_name, &input, unrestricted);
        match policy_verdict {
            PolicyVerdict::Deny { reason } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    &ctx.session_id,
                    None,
                    &start,
                    "denied",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::PolicyDenied { reason });
            }
            PolicyVerdict::RequireApproval { reason } => {
                let summary = input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(|c| format!("{tool_name}: {c}"))
                    .unwrap_or_else(|| format!("{tool_name}: {input}"));

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
                            &ctx.session_id,
                            None,
                            &start,
                            "rule_denied",
                        )
                        .await
                        .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                        return Err(ToolError::PolicyDenied {
                            reason: format!("{reason}: denied by permission rule/mode"),
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
                                &ctx.session_id,
                                None,
                                &start,
                                "approval_authority_missing",
                            )
                            .await
                            .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                            return Err(ToolError::PolicyDenied {
                                reason: format!(
                                    "{reason}: authenticated approval authority is unavailable"
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
                                scope_subject: approval_scope_subject(
                                    tool,
                                    &input,
                                    &authority.workspace,
                                ),
                            };
                            match self.approval_gate.request(&req).await {
                                ApprovalDecision::Approve => {}
                                ApprovalDecision::ApproveForSession => {
                                    self.session_approvals.insert(grant_key);
                                }
                                ApprovalDecision::ApprovePathForSession => {}
                                ApprovalDecision::Deny => {
                                    self.log_audit(
                                        audit_id,
                                        tool_name,
                                        &input,
                                        tool.permission_level(),
                                        turn_id,
                                        &ctx.session_id,
                                        None,
                                        &start,
                                        "approval_denied",
                                    )
                                    .await
                                    .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                                    return Err(ToolError::PolicyDenied {
                                        reason: format!("{reason}: denied by approval gate"),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            PolicyVerdict::Allow => {}
        }

        // D1-T12: shell networking is an explicit, per-call capability. It is
        // intentionally independent of the ordinary permission mode and of
        // session-wide tool grants: neither BypassAll nor an earlier approval
        // for one shell tool may silently turn networking on for a later call.
        let shell_network_requested = matches!(tool_name, "bash_exec" | "exec_command")
            && input
                .get("network_enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
        if shell_network_requested && !unrestricted {
            let Some(authority) = ctx.approval_authority.as_ref() else {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    &ctx.session_id,
                    None,
                    &start,
                    "network_approval_authority_missing",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::PolicyDenied {
                    reason: "shell network access requires an authenticated approval authority"
                        .into(),
                });
            };
            let command = input
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let request = ApprovalRequest {
                owner: fabric::ApprovalOwner::new(
                    authority.principal_id.clone(),
                    authority.thread_id.clone(),
                ),
                connection_id: authority.connection_id.clone(),
                turn_id: authority.turn_id,
                call_id: authority.call_id.clone(),
                workspace: authority.workspace.clone(),
                tool: tool_name.to_owned(),
                action_summary: format!("shell network access: {command}"),
                risk_level: "network".into(),
                detail: Some(input.to_string()),
                scope_subject: None,
            };
            if !matches!(
                self.approval_gate.request(&request).await,
                ApprovalDecision::Approve
                    | ApprovalDecision::ApproveForSession
                    | ApprovalDecision::ApprovePathForSession
            ) {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    &ctx.session_id,
                    None,
                    &start,
                    "network_approval_denied",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::PolicyDenied {
                    reason: "shell network access was denied by the approval gate".into(),
                });
            }
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
                    &ctx.session_id,
                    None,
                    &start,
                    "loop_blocked",
                )
                .await
                .map_err(|e| ToolError::AuditFailed(e.to_string()))?;
                return Err(ToolError::LoopBlocked {
                    reason: format!("{reason}. {suggestion}"),
                });
            }
            LoopVerdict::Escalate { reason } => {
                self.log_audit(
                    audit_id,
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    &ctx.session_id,
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
                    &ctx.session_id,
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
        let execution_descriptor = tool.execution_descriptor();
        let strategy = if unrestricted {
            ToolExecutionStrategy::InProcess
        } else if self.sandbox_profiles.is_none() {
            if tool_name == "bash_exec" {
                ToolExecutionStrategy::Sandboxed
            } else {
                ToolExecutionStrategy::InProcess
            }
        } else if execution_descriptor.is_some() {
            ToolExecutionStrategy::Sandboxed
        } else {
            resolve_strategy(tool_name, tool.permission_level())
        };

        let result = match strategy {
            ToolExecutionStrategy::Sandboxed | ToolExecutionStrategy::ExecdRequired => {
                // Command tools use SandboxExecutor; structured tools use the
                // injected filesystem-capable transport below.
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

                let workspace = ctx
                    .effective_workspace_policy()
                    .map_err(|reason| ToolError::PolicyDenied { reason })?;
                let trusted_working_dir = workspace.cwd().to_string_lossy().to_string();

                // S1 T13: resolve the default sandbox profile when profiles are
                // configured.
                let mut policy = if let Some(profiles) = self.sandbox_profiles.as_ref() {
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

                // Bash is deny-by-default even when the profile feature is
                // disabled or the selected trusted profile normally permits
                // networking. Materialize a workspace policy for the legacy
                // path so the backend receives an enforceable network bit.
                // Only the per-call approval above may clear that bit.
                if tool_name == "bash_exec" {
                    if policy.is_none() {
                        policy = Some(
                            resolve_profile(
                                &ProfileName::Workspace,
                                &workspace,
                                &SandboxProfiles::default(),
                            )
                            .map_err(|error| ToolError::PolicyDenied {
                                reason: format!(
                                    "default bash network sandbox could not be resolved (fail-closed): {error}"
                                ),
                            })?,
                        );
                    }
                    if let Some(policy) = policy.as_mut() {
                        policy.restrict_network = !shell_network_requested;
                    }
                }

                // A denied-network bash command must not degrade onto a
                // backend that cannot actually isolate the network. This is
                // stricter than the generic BestEffort profile behavior.
                if tool_name == "bash_exec" && !shell_network_requested {
                    let backend = self.sandbox.select_backend().ok_or_else(|| {
                        ToolError::ExecutionFailed(
                            "no sandbox backend is available for bash network isolation".into(),
                        )
                    })?;
                    if !backend.capabilities().network_isolation {
                        return Err(ToolError::PolicyDenied {
                            reason: format!(
                                "bash network is denied, but sandbox backend '{}' cannot enforce network isolation (fail-closed)",
                                backend.name()
                            ),
                        });
                    }
                }

                let sandbox_scratch = if tool_name == "bash_exec" {
                    Some(
                        tempfile::Builder::new()
                            .prefix("aletheon-sandbox-")
                            .tempdir()
                            .map_err(|error| {
                                ToolError::ExecutionFailed(format!(
                                    "could not create private sandbox scratch: {error}"
                                ))
                            })?,
                    )
                } else {
                    None
                };
                if let (Some(policy), Some(scratch)) = (policy.as_mut(), sandbox_scratch.as_ref()) {
                    policy.read_write_roots.push(scratch.path().to_path_buf());
                }
                let sandbox_config = SandboxConfig {
                    workspace,
                    environment: sandbox_command_environment(
                        trusted_working_dir,
                        sandbox_scratch.as_ref().map(tempfile::TempDir::path),
                    ),
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
                    sandbox_backend = Some(transport.backend_name().to_owned());
                    if execution_descriptor.is_none() && !transport.supports_tool(tool_name) {
                        return Err(ToolError::StructuredSandboxUnsupported {
                            tool: tool_name.to_owned(),
                        });
                    }
                    transport
                        .execute(
                            tool_name,
                            execution_descriptor.as_ref(),
                            input.clone(),
                            ctx,
                            &sandbox_config,
                        )
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

                    sandbox_backend = self
                        .sandbox
                        .select_backend()
                        .map(|backend| backend.name().to_owned());
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
                                patch_delta: None,
                            },
                        },
                        Err(e) => ToolResult {
                            content: format!("Sandbox execution failed: {e}"),
                            is_error: true,
                            metadata: ToolResultMeta {
                                execution_time_ms: 0,
                                truncated: false,
                                patch_delta: None,
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
                match kernel::chronos::SystemTimer
                    .timeout(Duration::from_secs(TOOL_TIMEOUT_SECS), execution)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => ToolResult {
                        content: format!("Tool '{tool_name}' timed out after {TOOL_TIMEOUT_SECS}s"),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: TOOL_TIMEOUT_SECS * 1000,
                            truncated: false,
                            patch_delta: None,
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
        let verdict_str = format!("{loop_verdict:?}");
        self.log_audit_with_backend(
            audit_id,
            tool_name,
            &input,
            tool.permission_level(),
            turn_id,
            &ctx.session_id,
            Some(&final_result),
            &start,
            &verdict_str,
            sandbox_backend,
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
        session_id: &str,
        result: Option<&ToolResult>,
        start: &fabric::MonoTime,
        verdict: &str,
    ) -> anyhow::Result<()> {
        self.log_audit_with_backend(
            audit_id, tool_name, input, level, turn_id, session_id, result, start, verdict, None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn log_audit_with_backend(
        &self,
        audit_id: fabric::AuditEventId,
        tool_name: &str,
        input: &serde_json::Value,
        level: PermissionLevel,
        turn_id: &str,
        session_id: &str,
        result: Option<&ToolResult>,
        start: &fabric::MonoTime,
        verdict: &str,
        sandbox_backend: Option<String>,
    ) -> anyhow::Result<()> {
        let category = if tool_name == "exec_command" {
            input
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(classify_command)
                .map(CommandEffect::risk_category)
                .unwrap_or_else(|| self.risk_classifier.classify(tool_name))
        } else {
            self.risk_classifier.classify(tool_name)
        };
        let record = AuditRecord {
            audit_id,
            timestamp: self.clock.wall_now(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_string(),
            tool_name: tool_name.to_string(),
            args: input.clone(),
            permission_level: level,
            risk_category: category,
            loop_verdict: verdict.to_string(),
            result_summary: result.map(|r| r.content.chars().take(200).collect()),
            is_error: result.map(|r| r.is_error).unwrap_or(false),
            sandbox_backend,
            elapsed_ms: self.clock.mono_now().0.saturating_sub(start.0),
        };
        self.audit_logger.log(record).await
    }

    pub fn metrics(&self) -> &super::loop_detector::LoopDetectorMetrics {
        &self.loop_detector.metrics
    }
}

fn approval_scope_subject(
    tool: &dyn Tool,
    input: &serde_json::Value,
    workspace: &fabric::WorkspacePolicy,
) -> Option<fabric::protocol::client::TransientApprovalScopeSubject> {
    let descriptor = tool.approval_descriptor(input, workspace).ok().flatten()?;
    if descriptor.mutation_targets.is_empty() {
        return None;
    }
    let mut path_candidates = workspace
        .writable_roots()
        .iter()
        .filter(|root| {
            descriptor
                .mutation_targets
                .iter()
                .all(|target| target.starts_with(root))
        })
        .cloned()
        .collect::<Vec<_>>();
    path_candidates.sort();
    path_candidates.dedup();
    if path_candidates.is_empty() {
        return None;
    }
    let version = 1u32;
    let digest_input = serde_json::to_vec(&(tool.name(), &path_candidates, version)).ok()?;
    Some(fabric::protocol::client::TransientApprovalScopeSubject {
        tool: tool.name().to_string(),
        path_candidates,
        subject_version: version,
        subject_sha256: format!("{:x}", Sha256::digest(digest_input)),
    })
}

#[cfg(test)]
mod tests;
