use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::audit::{AuditLogger, AuditRecord};
use super::loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
use super::output_guardrail::OutputGuardrail;
use super::policy::{PolicyEngine, PolicyVerdict};
use super::risk_classifier::RiskClassifier;
use fabric::sandbox::{SandboxConfig, SandboxExecutor, SandboxPreference};
use fabric::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

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
    clock: Arc<dyn fabric::Clock>,
}

impl ToolRunnerWithGuard {
    pub fn new(
        sandbox: SandboxExecutor,
        audit_logger: AuditLogger,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            sandbox,
            loop_detector: LoopDetector::new(LoopDetectorConfig::default()),
            output_guardrail: OutputGuardrail::with_defaults(),
            policy_engine: PolicyEngine::with_defaults(),
            audit_logger,
            risk_classifier: RiskClassifier::with_defaults(),
            clock,
        }
    }

    /// Create with a sandbox executor using Auto preference and the given backends.
    pub fn with_default_sandbox(
        audit_logger: AuditLogger,
        clock: Arc<dyn fabric::Clock>,
        backends: Vec<Box<dyn fabric::sandbox::SandboxBackend>>,
    ) -> Self {
        Self::new(
            SandboxExecutor::new(backends, SandboxPreference::Auto),
            audit_logger,
            clock,
        )
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
        let start = self.clock.mono_now();

        // 1. Policy check
        let policy_verdict = self.policy_engine.check(tool_name, &input);
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
                // In automated mode, deny L2+ that require approval
                if tool.permission_level() >= PermissionLevel::L2 {
                    self.log_audit(
                        tool_name,
                        &input,
                        tool.permission_level(),
                        turn_id,
                        None,
                        &start,
                        "requires_approval",
                    )
                    .await;
                    return Err(ToolError::PolicyDenied {
                        reason: format!(
                            "{}: {}",
                            reason, "L2+ requires approval in automated mode"
                        ),
                    });
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
            // Direct execution for L0 tools
            tool.execute(input.clone(), ctx).await
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

    async fn log_audit(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        level: PermissionLevel,
        turn_id: &str,
        result: Option<&ToolResult>,
        start: &fabric::MonoTime,
        verdict: &str,
    ) {
        let category = self.risk_classifier.classify(tool_name);
        let record = AuditRecord {
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
        let _ = self.audit_logger.log(record).await;
    }

    pub fn metrics(&self) -> &super::loop_detector::LoopDetectorMetrics {
        &self.loop_detector.metrics
    }
}
