use super::super::approval::{AutoApproveGate, AutoDenyGate};
use super::super::execpolicy::{Decision as ExecDecision, PrefixRule as ExecPrefixRule};
use super::*;
use ::contracts::tool::{
    ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult, ToolResultMeta,
};
use async_trait::async_trait;

#[test]
fn sandbox_environment_exposes_toolchain_identity_without_secrets() {
    let source = std::collections::BTreeMap::from([
        ("HOME", "/home/dev"),
        ("PATH", "/home/dev/.cargo/bin:/usr/bin"),
        ("DEEPSEEK_API_KEY", "secret"),
        ("RUSTC_WRAPPER", "/tmp/injector"),
    ]);
    let environment = sandbox_command_environment_with(
        "/work".into(),
        Some(std::path::Path::new("/scratch")),
        |key| {
            source
                .get(key)
                .map(|value| (*value).to_owned())
                .ok_or(std::env::VarError::NotPresent)
        },
    );

    assert_eq!(environment["PATH"], "/home/dev/.cargo/bin:/usr/bin");
    assert_eq!(environment["CARGO_HOME"], "/home/dev/.cargo");
    assert_eq!(environment["RUSTUP_HOME"], "/home/dev/.rustup");
    assert_eq!(environment["GIT_CONFIG_VALUE_0"], "/work");
    assert_eq!(environment["TMPDIR"], "/scratch");
    assert_eq!(environment["TMP"], "/scratch");
    assert_eq!(environment["TEMP"], "/scratch");
    assert!(!environment.contains_key("DEEPSEEK_API_KEY"));
    assert!(!environment.contains_key("RUSTC_WRAPPER"));
}
use ::contracts::{PermissionContext, PermissionMode};
use kernel::chronos::TestClock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A dummy L2 tool used to exercise the approval gate path.
/// Named "bash_exec" so the policy engine's `rm -rf *` rule triggers RequireApproval.
struct DummyL2Tool;

struct StructuredL1Tool {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

struct DescriptorTool {
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
        sink: &mut ::contracts::ToolEventSink,
    ) {
        assert!(sink.progress(::contracts::ToolProgress::Text("phase-1".into())));
        assert!(sink.progress(::contracts::ToolProgress::Structured(
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

#[async_trait]
impl Tool for DescriptorTool {
    fn name(&self) -> &str {
        "module_build"
    }

    fn description(&self) -> &str {
        "host-described structured build"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L2
    }

    fn execution_descriptor(&self) -> Option<::contracts::tool::ToolExecutionDescriptor> {
        Some(::contracts::tool::ToolExecutionDescriptor::ModuleBuild)
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult {
            content: "in-process build".into(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self {
            calls: Arc::clone(&self.calls),
        })
    }
}

struct MockStructuredSandbox {
    calls: Arc<AtomicUsize>,
    expected_tool: &'static str,
    expected_descriptor: Option<::contracts::tool::ToolExecutionDescriptor>,
}

#[async_trait]
impl StructuredToolSandbox for MockStructuredSandbox {
    fn supports_tool(&self, tool_name: &str) -> bool {
        tool_name == self.expected_tool
    }

    async fn execute(
        &self,
        tool_name: &str,
        descriptor: Option<&::contracts::tool::ToolExecutionDescriptor>,
        input: serde_json::Value,
        _context: &ToolContext,
        sandbox: &SandboxConfig,
    ) -> Result<ToolResult, String> {
        assert_eq!(tool_name, self.expected_tool);
        assert_eq!(descriptor, self.expected_descriptor.as_ref());
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

struct CountingApproveGate(Arc<AtomicUsize>);

#[async_trait]
impl ApprovalGate for CountingApproveGate {
    async fn request(&self, _request: &ApprovalRequest) -> ApprovalDecision {
        self.0.fetch_add(1, Ordering::SeqCst);
        ApprovalDecision::ApproveForSession
    }
}

fn make_ctx() -> ToolContext {
    ToolContext {
        approval_authority: Some(::contracts::ToolApprovalAuthority {
            principal_id: ::contracts::PrincipalId("test".into()),
            connection_id: ::contracts::ConnectionId::new(),
            thread_id: ::contracts::ThreadId("test-session".into()),
            turn_id: ::contracts::TurnId::new(),
            call_id: "test-call".into(),
            workspace: ::contracts::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![])
                .unwrap(),
            granted_scope: ::contracts::CapabilityScope::default(),
            permission_mode: ::contracts::permission::HostPermissionMode::Safe,
        }),
        agent: None,
        working_dir: std::path::PathBuf::from("/tmp"),
        session_id: "test-session".into(),
        clock: test_clock(),
        turn_event_sender: None,
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
            .with_sandbox_profiles(::contracts::SandboxProfiles::default());

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
        .with_sandbox_profiles(::contracts::SandboxProfiles::default())
        .with_structured_sandbox(Arc::new(MockStructuredSandbox {
            calls: Arc::clone(&transport_calls),
            expected_tool: "file_write",
            expected_descriptor: None,
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
    let profiles = ::contracts::SandboxProfiles {
        default_profile: "missing-trusted-profile".into(),
        ..::contracts::SandboxProfiles::default()
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
            .with_sandbox_profiles(::contracts::SandboxProfiles::default())
            .with_structured_sandbox(Arc::new(MockStructuredSandbox {
                calls: Arc::clone(&transport_calls),
                expected_tool: name,
                expected_descriptor: None,
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
async fn structured_transport_receives_host_descriptor_separately_from_model_input() {
    let in_process_calls = Arc::new(AtomicUsize::new(0));
    let transport_calls = Arc::new(AtomicUsize::new(0));
    let tool = DescriptorTool {
        calls: Arc::clone(&in_process_calls),
    };
    let mut runner = make_runner(Arc::new(AutoApproveGate))
        .with_sandbox_profiles(::contracts::SandboxProfiles::default())
        .with_structured_sandbox(Arc::new(MockStructuredSandbox {
            calls: Arc::clone(&transport_calls),
            expected_tool: "module_build",
            expected_descriptor: Some(::contracts::tool::ToolExecutionDescriptor::ModuleBuild),
        }));

    let result = runner
        .execute_tool(
            &tool,
            // A model-controlled field that resembles the typed descriptor
            // must remain ordinary input and cannot replace host identity.
            serde_json::json!({
                "content": "written",
                "descriptor": {"kind": "module_load"}
            }),
            &make_ctx(),
            "descriptor-trust-boundary-turn",
        )
        .await
        .unwrap();

    assert_eq!(result.content, "sandbox wrote artifact");
    assert_eq!(in_process_calls.load(Ordering::SeqCst), 0);
    assert_eq!(transport_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guarded_streaming_executes_override_once_and_preserves_terminal() {
    let mut runner = make_runner(Arc::new(AutoApproveGate));
    let (mut sink, mut rx) = ::contracts::tool_event_channel();

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
        Some(::contracts::ToolExecutionEvent::Progress(
            ::contracts::ToolProgress::Text(_)
        ))
    ));
    assert!(matches!(
        rx.recv().await,
        Some(::contracts::ToolExecutionEvent::Progress(
            ::contracts::ToolProgress::Structured(_)
        ))
    ));
    assert!(matches!(
        rx.recv().await,
        Some(::contracts::ToolExecutionEvent::Terminal(Ok(_)))
    ));
}

#[tokio::test]
async fn bash_sandbox_streams_multiple_lines_and_one_terminal() {
    let mut runner = make_runner(Arc::new(AutoApproveGate));
    let (mut sink, mut rx) = ::contracts::tool_event_channel();
    let report = runner
        .execute_tool_streaming_report(
            &DummyL2Tool,
            serde_json::json!({
                "command": "printf 'alpha\\n'; sleep 0.05; printf 'beta\\n'",
                "network_enabled": true
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
            ::contracts::ToolExecutionEvent::Progress(::contracts::ToolProgress::Text(line)) => {
                progress.push(line);
            }
            ::contracts::ToolExecutionEvent::Terminal(_) => terminals += 1,
            _ => {}
        }
    }
    assert_eq!(progress, ["alpha", "beta"]);
    assert_eq!(terminals, 1);
}

#[tokio::test]
async fn sandbox_profile_applied_event_carries_policy_and_principal() {
    let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
    let mut events = bus.subscribe_channel(::contracts::SchemaId(
        ::contracts::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
    ));
    let mut runner = make_runner(Arc::new(AutoApproveGate))
        .with_sandbox_profiles(::contracts::SandboxProfiles::default())
        .with_event_bus(bus);

    runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({"command": "printf ok", "network_enabled": true}),
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
    let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
    let mut events = bus.subscribe_channel(::contracts::SchemaId(
        ::contracts::SchemaId::EVENT_SANDBOX_VIOLATION_V1.into(),
    ));
    let profiles = ::contracts::SandboxProfiles {
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
        ::contracts::WorkspacePolicy::from_resolved_roots(
            workspace.path().to_path_buf(),
            Vec::new(),
        )
        .unwrap();
    let profiles = ::contracts::SandboxProfiles {
        default_profile: "guarded".into(),
        profiles: std::collections::BTreeMap::from([(
            "guarded".into(),
            ::contracts::SandboxProfileConfig {
                extends: Some("workspace".into()),
                restrict_network: None,
                read_only: Vec::new(),
                read_write: Vec::new(),
                deny: vec!["**/.env".into()],
            },
        )]),
    };
    let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
    let mut events = bus.subscribe_channel(::contracts::SchemaId(
        ::contracts::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
    ));
    let mut runner = make_runner(Arc::new(AutoApproveGate))
        .with_sandbox_profiles(profiles)
        .with_event_bus(bus);

    runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({"command": "printf ok", "network_enabled": true}),
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
    let profiles = ::contracts::SandboxProfiles {
        default_profile: "overflow".into(),
        profiles: std::collections::BTreeMap::from([(
            "overflow".into(),
            ::contracts::SandboxProfileConfig {
                extends: Some("workspace".into()),
                restrict_network: None,
                read_only: Vec::new(),
                read_write: Vec::new(),
                deny: (0..=::contracts::DENY_GLOB_MAX_ENTRIES)
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
    let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
    let mut events = bus.subscribe_channel(::contracts::SchemaId(
        ::contracts::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1.into(),
    ));
    let mut runner = make_runner(Arc::new(AutoApproveGate))
        .with_sandbox_profiles(::contracts::SandboxProfiles::default())
        .with_event_bus(bus);
    let parent_ctx = make_ctx();
    let mut child_ctx = make_ctx();
    child_ctx.agent = Some(::contracts::AgentToolContext {
        caller_root_agent_id: ::contracts::AgentId::new(),
        parent_agent_id: ::contracts::AgentId::new(),
        parent_process_id: ::contracts::ProcessId::new(),
        delegator_authority: None,
    });

    runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({"command": "printf parent", "network_enabled": true}),
            &parent_ctx,
            "parent-profile-turn",
        )
        .await
        .unwrap();
    runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({"command": "printf child", "network_enabled": true}),
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
        .execute_tool(
            &tool,
            serde_json::json!({
                "command": "rm -rf /tmp/test",
                "network_enabled": true
            }),
            &make_ctx(),
            "t1",
        )
        .await;
    assert!(result.is_err(), "AutoDenyGate should deny L2 tool");
    match result.unwrap_err() {
        ToolError::PolicyDenied { reason } => {
            assert!(
                reason.contains("denied by approval gate"),
                "reason: {reason}"
            );
        }
        other => panic!("Expected PolicyDenied, got: {other:?}"),
    }
}

#[tokio::test]
async fn bash_network_request_requires_authenticated_authority() {
    let mut runner = make_runner(Arc::new(AutoApproveGate));
    let mut context = make_ctx();
    context.approval_authority = None;

    let result = runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({
                "command": "printf network",
                "network_enabled": true
            }),
            &context,
            "network-authority-turn",
        )
        .await;

    assert!(matches!(result, Err(ToolError::PolicyDenied { ref reason })
        if reason.contains("authenticated approval authority")));
}

#[tokio::test]
async fn bash_network_approval_is_per_call_even_for_session_decision() {
    let approvals = Arc::new(AtomicUsize::new(0));
    let mut runner = make_runner(Arc::new(CountingApproveGate(Arc::clone(&approvals))));
    let input = serde_json::json!({
        "command": "printf network",
        "network_enabled": true
    });

    runner
        .execute_tool(&DummyL2Tool, input.clone(), &make_ctx(), "network-turn-1")
        .await
        .expect("first explicitly approved call");
    runner
        .execute_tool(&DummyL2Tool, input, &make_ctx(), "network-turn-2")
        .await
        .expect("second explicitly approved call");

    assert_eq!(approvals.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn bash_without_network_request_fails_closed_on_nonisolating_backend() {
    let mut runner = make_runner(Arc::new(AutoApproveGate));
    let result = runner
        .execute_tool(
            &DummyL2Tool,
            serde_json::json!({"command": "printf offline"}),
            &make_ctx(),
            "network-denied-turn",
        )
        .await;

    assert!(matches!(result, Err(ToolError::PolicyDenied { ref reason })
        if reason.contains("cannot enforce network isolation") && reason.contains("fail-closed")));
}

#[tokio::test]
async fn l2_approved_by_gate_runs() {
    let mut runner = make_runner(Arc::new(AutoApproveGate));
    let tool = DummyL2Tool;
    let result = runner
        .execute_tool(
            &tool,
            serde_json::json!({
                "command": "rm -rf /tmp/test",
                "network_enabled": true
            }),
            &make_ctx(),
            "t1",
        )
        .await;
    assert!(
        matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
        "AutoApproveGate should pass policy before output validation: {result:?}"
    );
}

#[tokio::test]
async fn bypass_all_does_not_bypass_explicit_network_approval() {
    // BypassAll may bypass the ordinary tool policy, but it is not an
    // authenticated grant of outbound network authority.
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
        .execute_tool(
            &tool,
            serde_json::json!({
                "command": "rm -rf /tmp/test",
                "network_enabled": true
            }),
            &make_ctx(),
            "t1",
        )
        .await;
    assert!(matches!(result, Err(ToolError::PolicyDenied { ref reason })
        if reason.contains("network access was denied")));
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
                "reason: {reason}"
            );
        }
        other => panic!("Expected PolicyDenied, got: {other:?}"),
    }
}

#[tokio::test]
async fn runner_uses_execpolicy_for_deny() {
    // Build an execpolicy that forbids "bash_exec" entirely.
    let mut policy = ExecPolicy::new();
    policy.add_rule(ExecPrefixRule::new("bash_exec", ExecDecision::Forbidden));

    let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
    let mut runner =
        ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock()).with_policy(policy);

    let tool = DummyL2Tool;
    let result = runner
        .execute_tool(&tool, make_input_rm(), &make_ctx(), "t1")
        .await;
    assert!(result.is_err(), "execpolicy should deny bash_exec");
    match result.unwrap_err() {
        ToolError::PolicyDenied { reason } => {
            assert!(reason.contains("Policy forbids"), "reason: {reason}");
        }
        other => panic!("Expected PolicyDenied, got: {other:?}"),
    }
}

#[path = "tests/runner_tail_tests.rs"]
mod tail_tests;
