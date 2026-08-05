use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use corpus::{security::AuditLogger, CorpusToolExecutor, ToolRegistry, ToolRunnerWithGuard};
use fabric::tool::{PermissionLevel, ToolCachePolicy};
use fabric::types::admission::RiskLevel;
use fabric::{
    BudgetRequest, CapabilityAuthority, CapabilityCall, CapabilityId, CapabilityRequest,
    CapabilityScope, ExecutionPermit, InvocationControl, MonoDeadline, MonoTime, OperationId,
    PrincipalId, ProcessId, Registry, SandboxDecision, SandboxRequirement, Tool, ToolContext,
    ToolResult, ToolResultMeta,
};
use kernel::{capability::ToolExecutor, chronos::TestClock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct CountingTool {
    calls: Arc<AtomicUsize>,
    cache_enabled: bool,
    emits_patch: bool,
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
    fn cache_policy(&self) -> ToolCachePolicy {
        if self.cache_enabled {
            ToolCachePolicy::PerTurn
        } else {
            ToolCachePolicy::Never
        }
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult {
            content: "counted".into(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: 7,
                truncated: false,
                patch_delta: self.emits_patch.then(|| fabric::PatchDelta {
                    applied: vec![],
                    failed: vec![],
                    files_changed: vec![],
                    ..Default::default()
                }),
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
            connection_id: fabric::ConnectionId::new(),
            thread_id: fabric::ThreadId("session-1".into()),
            turn_id: fabric::TurnId::new(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots(std::env::temp_dir(), vec![])
                .unwrap(),
            session_id: "session-1".into(),
            working_dir: std::env::temp_dir(),
            permission_mode: fabric::permission::HostPermissionMode::Safe,
        },
        control: InvocationControl {
            cancel: CancellationToken::new(),
            turn_event_sender: None,
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

async fn fixture_with_options(
    cache_enabled: bool,
    emits_patch: bool,
) -> (
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
            cache_enabled,
            emits_patch,
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

async fn fixture_with_cache(
    cache_enabled: bool,
) -> (
    CorpusToolExecutor,
    CapabilityRequest,
    ExecutionPermit,
    Arc<AtomicUsize>,
    tempfile::TempDir,
) {
    fixture_with_options(cache_enabled, !cache_enabled).await
}

async fn fixture() -> (
    CorpusToolExecutor,
    CapabilityRequest,
    ExecutionPermit,
    Arc<AtomicUsize>,
    tempfile::TempDir,
) {
    fixture_with_cache(false).await
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
    assert_eq!(result.patch_delta, Some(fabric::PatchDelta::default()));
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
        approval_authority: None,
        agent: None,
        working_dir: temp.path().into(),
        session_id: "s".into(),
        clock,
        turn_event_sender: None,
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
        cache_enabled: false,
        emits_patch: true,
    };
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(
        AuditLogger::new(temp.path().to_path_buf()).unwrap(),
        clock.clone(),
    );
    let ctx = ToolContext {
        approval_authority: None,
        agent: None,
        working_dir: temp.path().into(),
        session_id: "s".into(),
        clock,
        turn_event_sender: None,
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

#[tokio::test]
async fn declared_read_only_cache_reauthorizes_audits_and_matches_streaming() {
    let (executor, request, permit, calls, temp) = fixture_with_cache(true).await;
    let first = executor.execute_with_permit(&request, &permit).await;
    assert!(!first.is_error, "{}", first.output);
    assert!(!first.served_from_cache);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let mut second_request = request.clone();
    second_request.call.call_id = "call-2".into();
    let (mut sink, mut events) = fabric::tool_event_channel();
    let second = executor
        .execute_streaming_with_permit(&second_request, &permit, &mut sink)
        .await;
    assert!(!second.is_error, "{}", second.output);
    assert!(second.served_from_cache);
    assert_eq!(second.call_id, "call-2");
    assert_eq!(second.usage.permit_id, permit.id);
    assert_eq!(second.usage.wall_time_ms, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        events.try_recv().unwrap(),
        fabric::ToolExecutionEvent::Terminal(Ok(_))
    ));

    let audit_records = std::fs::read_to_string(temp.path().join("audit.jsonl")).unwrap();
    let records = audit_records
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0]["audit_id"], records[1]["audit_id"]);
    assert!(records[1]["loop_verdict"]
        .as_str()
        .unwrap()
        .starts_with("cache_hit:"));
    assert_eq!(
        second.audit_id.unwrap().0.to_string(),
        records[1]["audit_id"]
    );

    let metrics = executor.read_only_cache_metrics();
    let tool_metrics = metrics.get("counting_tool").unwrap();
    assert_eq!(tool_metrics.miss_total, 1);
    assert_eq!(tool_metrics.hit_total, 1);
}

#[tokio::test]
async fn patch_producing_result_is_never_cached_even_if_tool_declares_policy() {
    let (executor, request, permit, calls, _temp) = fixture_with_options(true, true).await;
    let first = executor.execute_with_permit(&request, &permit).await;
    let second = executor.execute_with_permit(&request, &permit).await;
    assert!(!first.served_from_cache);
    assert!(!second.served_from_cache);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_cap_002_cancelled_invocation_fails_closed_before_tool_lookup_and_emits_terminal() {
    let (executor, request, permit, calls, _temp) = fixture().await;
    request.control.cancel.cancel();

    let (mut sink, mut events) = fabric::tool_event_channel();
    let result = executor
        .execute_streaming_with_permit(&request, &permit, &mut sink)
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("cancelled before execution"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        events.recv().await,
        Some(fabric::ToolExecutionEvent::Terminal(Err(
            fabric::ToolExecutionError::Cancelled(message)
        ))) if message == "capability invocation cancelled before execution"
    ));
    assert!(sink.terminal_sent());
}

#[tokio::test]
async fn streaming_rejected_permits_emit_a_failed_terminal_without_tool_execution() {
    let (executor, request, mut permit, calls, _temp) = fixture().await;
    permit.sandbox = SandboxDecision::Unavailable;

    let (mut sink, mut events) = fabric::tool_event_channel();
    let result = executor
        .execute_streaming_with_permit(&request, &permit, &mut sink)
        .await;

    assert!(result.is_error);
    assert!(result
        .output
        .contains("permit expired or sandbox unavailable"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        events.recv().await,
        Some(fabric::ToolExecutionEvent::Terminal(Err(
            fabric::ToolExecutionError::Failed(message)
        ))) if message == "permit expired or sandbox unavailable"
    ));
    assert!(sink.terminal_sent());
}

#[tokio::test]
async fn mutation_delta_retains_invocation_permit_and_audit_linkage() {
    let (executor, request, permit, calls, _temp) = fixture_with_options(false, true).await;

    let result = executor.execute_with_permit(&request, &permit).await;

    assert!(!result.is_error, "{}", result.output);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.call_id, request.call.call_id);
    assert_eq!(result.usage.permit_id, permit.id);
    assert!(
        result.audit_id.is_some(),
        "mutation needs an audit identity"
    );
    assert_eq!(result.patch_delta, Some(fabric::PatchDelta::default()));
    assert!(!result.served_from_cache);
}
