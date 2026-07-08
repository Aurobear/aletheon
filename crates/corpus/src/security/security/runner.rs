use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

use super::approval::{ApprovalDecision, ApprovalGate, ApprovalRequest, AutoDenyGate};
use super::audit::{AuditLogger, AuditRecord};
use super::loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
use super::output_guardrail::OutputGuardrail;
use super::policy::{PolicyEngine, PolicyVerdict};
use super::risk_classifier::RiskClassifier;
use crate::sandbox::{SandboxConfig, SandboxExecutor, SandboxPreference};
use base::execpolicy::{Decision as ExecDecision, Policy as ExecPolicy};
use base::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use base::{PermissionBehavior, PermissionContext};

#[derive(Debug)]
pub enum ToolError {
    PolicyDenied { reason: String },
    LoopBlocked { reason: String },
    EscalateToHuman { reason: String },
    InterruptTurn { reason: String },
    MaxRetriesExceeded,
    ExecutionFailed(String),
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
        }
    }
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
    /// Tool names approved for the rest of the session (via ApproveForSession).
    session_approvals: std::collections::HashSet<String>,
    /// Permission context for mode/rule-based pre-approval.
    permission_ctx: PermissionContext,
    /// Independent execpolicy engine. When set, takes precedence over the inline PolicyEngine.
    exec_policy: Option<ExecPolicy>,
}

impl ToolRunnerWithGuard {
    pub fn new(sandbox: SandboxExecutor, audit_logger: AuditLogger) -> Self {
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
        }
    }

    /// Create with default sandbox (Auto preference).
    pub fn with_default_sandbox(audit_logger: AuditLogger) -> Self {
        use crate::sandbox::SandboxPreference;
        Self::new(SandboxExecutor::new(SandboxPreference::Auto), audit_logger)
    }

    /// Create with an explicit sandbox preference.
    pub fn with_sandbox_preference(
        audit_logger: AuditLogger,
        preference: SandboxPreference,
    ) -> Self {
        Self::new(SandboxExecutor::new(preference), audit_logger)
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
            let eval = policy.check(&cmd, base::execpolicy::default_heuristics);
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
        let tool_name = tool.name();
        let start = Instant::now();

        // 1. Policy check
        let policy_verdict = self.check_policy(tool_name, &input);
        match policy_verdict {
            PolicyVerdict::Deny { reason } => {
                self.log_audit(
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "denied",
                )
                .await;
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
                                tool_name,
                                &input,
                                tool.permission_level(),
                                turn_id,
                                None,
                                &start,
                                "rule_denied",
                            )
                            .await;
                            return Err(ToolError::PolicyDenied {
                                reason: format!("{}: denied by permission rule/mode", reason),
                            });
                        }
                        PermissionBehavior::Ask => {
                            // Fall through to existing approval-gate flow.
                            if self.session_approvals.contains(tool_name) {
                                // Previously approved-for-session; allow.
                            } else {
                                let req = ApprovalRequest {
                                    tool: tool_name.to_string(),
                                    action_summary: summary,
                                    risk_level: format!("{:?}", tool.permission_level()),
                                    detail: Some(input.to_string()),
                                };
                                match self.approval_gate.request(&req).await {
                                    ApprovalDecision::Approve => {}
                                    ApprovalDecision::ApproveForSession => {
                                        self.session_approvals.insert(tool_name.to_string());
                                    }
                                    ApprovalDecision::Deny => {
                                        self.log_audit(
                                            tool_name,
                                            &input,
                                            tool.permission_level(),
                                            turn_id,
                                            None,
                                            &start,
                                            "approval_denied",
                                        )
                                        .await;
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
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "loop_blocked",
                )
                .await;
                return Err(ToolError::LoopBlocked {
                    reason: format!("{}. {}", reason, suggestion),
                });
            }
            LoopVerdict::Escalate { reason } => {
                self.log_audit(
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "escalated",
                )
                .await;
                return Err(ToolError::EscalateToHuman {
                    reason: reason.clone(),
                });
            }
            LoopVerdict::InterruptTurn { reason, .. } => {
                self.log_audit(
                    tool_name,
                    &input,
                    tool.permission_level(),
                    turn_id,
                    None,
                    &start,
                    "interrupted",
                )
                .await;
                return Err(ToolError::InterruptTurn {
                    reason: reason.clone(),
                });
            }
        }

        // 3. Execute tool (with optional sandbox for L1+)
        let result = if tool.permission_level() >= PermissionLevel::L1 {
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

            let sandbox_config = SandboxConfig {
                working_dir: ctx.working_dir.to_string_lossy().to_string(),
                env_vars: std::collections::HashMap::new(),
            };

            match self
                .sandbox
                .run(cmd, &sandbox_config, Duration::from_secs(30))
                .await
            {
                Ok(sandbox_result) => ToolResult {
                    content: format!("{}\n{}", sandbox_result.stdout, sandbox_result.stderr)
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
        } else {
            // Direct execution for L0 tools with timeout
            const L0_TIMEOUT_SECS: u64 = 60;
            match tokio::time::timeout(
                Duration::from_secs(L0_TIMEOUT_SECS),
                tool.execute(input.clone(), ctx),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => ToolResult {
                    content: format!("Tool '{}' timed out after {}s", tool_name, L0_TIMEOUT_SECS),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: L0_TIMEOUT_SECS * 1000,
                        truncated: false,
                    },
                },
            }
        };

        // 4. Output guardrail validation with retries
        let mut final_result = result;
        for retry in 0..self.output_guardrail.max_retries {
            match self.output_guardrail.validate(&final_result).await {
                Ok(()) => break,
                Err(e) => {
                    warn!(tool = tool_name, retry = retry, error = ?e, "Output validation failed");
                    if retry < self.output_guardrail.max_retries - 1 {
                        final_result = tool.execute(input.clone(), ctx).await;
                    } else {
                        warn!(
                            tool = tool_name,
                            "Max retries exceeded for output validation"
                        );
                    }
                }
            }
        }

        // 5. Loop detector post-check
        self.loop_detector
            .post_check(tool_name, &input, &final_result, turn_id);

        // 6. Audit log
        let verdict_str = format!("{:?}", loop_verdict);
        self.log_audit(
            tool_name,
            &input,
            tool.permission_level(),
            turn_id,
            Some(&final_result),
            &start,
            &verdict_str,
        )
        .await;

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
        tool_name: &str,
        input: &serde_json::Value,
        level: PermissionLevel,
        turn_id: &str,
        result: Option<&ToolResult>,
        start: &Instant,
        verdict: &str,
    ) {
        let category = self.risk_classifier.classify(tool_name);
        let record = AuditRecord {
            timestamp: chrono::Utc::now(),
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
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
        let _ = self.audit_logger.log(record).await;
    }

    pub fn metrics(&self) -> &super::loop_detector::LoopDetectorMetrics {
        &self.loop_detector.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::super::approval::{AutoApproveGate, AutoDenyGate};
    use super::*;
    use async_trait::async_trait;
    use base::execpolicy::{Decision as ExecDecision, PrefixRule as ExecPrefixRule};
    use base::tool::{
        ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult,
        ToolResultMeta,
    };
    use base::{PermissionContext, PermissionMode};

    /// A dummy L2 tool used to exercise the approval gate path.
    /// Named "bash_exec" so the policy engine's `rm -rf *` rule triggers RequireApproval.
    struct DummyL2Tool;

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

    fn make_runner(gate: Arc<dyn ApprovalGate>) -> ToolRunnerWithGuard {
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        ToolRunnerWithGuard::with_default_sandbox(audit_logger).with_approval_gate(gate)
    }

    fn make_input_rm() -> serde_json::Value {
        serde_json::json!({ "command": "rm -rf /tmp/test" })
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test-session".into(),
        }
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
            result.is_ok(),
            "AutoApproveGate should allow L2 tool: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().content, "ok");
    }

    #[tokio::test]
    async fn bypass_all_approves_l2() {
        // BypassAll mode should allow L2 tool without any approval gate prompt.
        let ctx = PermissionContext {
            mode: PermissionMode::BypassAll,
            ..Default::default()
        };
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger)
            .with_approval_gate(Arc::new(AutoDenyGate))
            .with_permission_context(ctx);
        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            result.is_ok(),
            "BypassAll mode should allow L2 tool even with AutoDenyGate: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().content, "ok");
    }

    #[tokio::test]
    async fn plan_mode_denies_dangerous() {
        // Plan mode should deny L2 (dangerous) tool, audit as "rule_denied".
        let ctx = PermissionContext {
            mode: PermissionMode::Plan,
            ..Default::default()
        };
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger)
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
        let mut runner =
            ToolRunnerWithGuard::with_default_sandbox(audit_logger).with_policy(policy);

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
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger)
            .with_approval_gate(Arc::new(AutoApproveGate))
            .with_policy(policy);

        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            result.is_ok(),
            "execpolicy Prompt + AutoApproveGate should allow: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn runner_no_execpolicy_falls_back_to_policy_engine() {
        // Without with_policy(), the inline PolicyEngine is used.
        let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger)
            .with_approval_gate(Arc::new(AutoApproveGate));

        let tool = DummyL2Tool;
        let result = runner
            .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
            .await;
        assert!(
            result.is_ok(),
            "Inline PolicyEngine + AutoApproveGate should allow bash_exec: {:?}",
            result.err()
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
