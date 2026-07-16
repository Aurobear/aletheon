use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use aletheon_kernel::{capability::ToolExecutor, chronos::TestClock};
use async_trait::async_trait;
use corpus::{security::AuditLogger, CorpusToolExecutor, ToolRegistry, ToolRunnerWithGuard};
use fabric::tool::PermissionLevel;
use fabric::types::admission::RiskLevel;
use fabric::{
    BudgetRequest, CapabilityAuthority, CapabilityCall, CapabilityId, CapabilityRequest,
    CapabilityScope, ExecutionPermit, InvocationControl, MonoDeadline, MonoTime, OperationId,
    PrincipalId, ProcessId, Registry, SandboxDecision, SandboxRequirement, Tool, ToolContext,
    ToolResult, ToolResultMeta,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct CountingTool {
    calls: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct EmptyCountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for EmptyCountingTool {
    fn name(&self) -> &str {
        "empty_counting_tool"
    }
    fn description(&self) -> &str {
        "returns rejected empty output"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult {
            content: String::new(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counting_tool"
    }
    fn description(&self) -> &str {
        "counts executions"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult {
            content: "counted".into(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: 7,
                truncated: false,
            },
        }
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}

fn request(operation_id: OperationId, process_id: ProcessId) -> CapabilityRequest {
    CapabilityRequest {
        call: CapabilityCall {
            operation_id,
            process_id,
            name: "counting_tool".into(),
            input: serde_json::json!({}),
            call_id: "call-1".into(),
            deadline: None,
        },
        authority: CapabilityAuthority {
            agent: None,
            principal: PrincipalId("test".into()),
            action: "execute".into(),
            requested_scope: CapabilityScope::default(),
            risk: RiskLevel::ReadOnly,
            budget: Some(BudgetRequest {
                max_tokens: None,
                max_cost_micro: None,
            }),
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
            session_id: "session-1".into(),
            working_dir: std::env::temp_dir(),
        },
        control: InvocationControl {
            cancel: CancellationToken::new(),
        },
    }
}

fn permit(operation_id: OperationId, process_id: ProcessId) -> ExecutionPermit {
    ExecutionPermit {
        id: fabric::PermitId::new(),
        operation_id,
        process_id,
        capability: CapabilityId("counting_tool".into()),
        granted_scope: CapabilityScope::default(),
        expires_at: MonoDeadline::after(MonoTime(0), 10_000),
        sandbox: SandboxDecision::NotApplicable,
        budget_reservation: None,
        lease: None,
    }
}

async fn fixture() -> (
    CorpusToolExecutor,
    CapabilityRequest,
    ExecutionPermit,
    Arc<AtomicUsize>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(0, 0));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(CountingTool {
            calls: calls.clone(),
        }))
        .unwrap();
    let runner = ToolRunnerWithGuard::with_default_sandbox(
        AuditLogger::new(temp.path().join("audit.jsonl")).unwrap(),
        clock.clone(),
    );
    let executor = CorpusToolExecutor::new(
        Arc::new(tokio::sync::Mutex::new(registry)),
        Arc::new(tokio::sync::Mutex::new(runner)),
        clock,
    );
    let operation_id = OperationId::new();
    let process_id = ProcessId::new();
    (
        executor,
        request(operation_id, process_id),
        permit(operation_id, process_id),
        calls,
        temp,
    )
}

#[tokio::test]
async fn mismatched_permit_fails_before_tool_lookup() {
    let (executor, request, mut permit, calls, _temp) = fixture().await;
    permit.operation_id = OperationId::new();
    let result = executor.execute_with_permit(&request, &permit).await;
    assert!(result.is_error);
    assert!(result.output.contains("permit does not bind request"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(result.usage.permit_id, permit.id);
}

#[tokio::test]
async fn guarded_tool_executes_once_with_durable_audit_identity() {
    let (executor, request, permit, calls, temp) = fixture().await;
    let result = executor.execute_with_permit(&request, &permit).await;
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(result.output, "counted");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.usage.permit_id, permit.id);
    assert_eq!(result.usage.wall_time_ms, 7);
    assert_eq!(result.usage.output_bytes, 7);
    let audit_id = result.audit_id.expect("audit id");
    let line = std::fs::read_to_string(temp.path().join("audit.jsonl")).unwrap();
    let record: serde_json::Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
    assert_eq!(record["audit_id"], serde_json::to_value(audit_id).unwrap());
}

#[tokio::test]
async fn sandbox_required_and_expired_permits_fail_closed() {
    let (executor, request, mut permit, calls, _temp) = fixture().await;
    permit.sandbox = SandboxDecision::Required;
    let result = executor.execute_with_permit(&request, &permit).await;
    assert!(result.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    permit.sandbox = SandboxDecision::NotApplicable;
    permit.expires_at = MonoDeadline::after(MonoTime(0), 0);
    let result = executor.execute_with_permit(&request, &permit).await;
    assert!(result.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn output_rejection_does_not_repeat_side_effect() {
    let temp = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(0, 0));
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = EmptyCountingTool {
        calls: calls.clone(),
    };
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(
        AuditLogger::new(temp.path().join("audit.jsonl")).unwrap(),
        clock.clone(),
    );
    let ctx = ToolContext {
        agent: None,
        working_dir: temp.path().into(),
        session_id: "s".into(),
        clock,
    };
    let report = runner
        .execute_tool_report(&tool, serde_json::json!({}), &ctx, "t")
        .await;
    assert!(matches!(
        report.result,
        Err(corpus::security::runner::ToolError::OutputRejected(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unwritable_audit_path_fails_execution() {
    let temp = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::new(0, 0));
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = CountingTool {
        calls: calls.clone(),
    };
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(
        AuditLogger::new(temp.path().to_path_buf()).unwrap(),
        clock.clone(),
    );
    let ctx = ToolContext {
        agent: None,
        working_dir: temp.path().into(),
        session_id: "s".into(),
        clock,
    };
    let report = runner
        .execute_tool_report(&tool, serde_json::json!({}), &ctx, "t")
        .await;
    assert!(matches!(
        report.result,
        Err(corpus::security::runner::ToolError::AuditFailed(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
