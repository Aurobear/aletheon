use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use corpus::{
    ActivationRequest, CorpusError, CorpusRetryDisposition, CorpusService, DefaultCorpusService,
    ExtensionCatalog, ExtensionDescriptor, ExtensionGrant, ExtensionId, ExtensionKind,
    GovernedInvocation,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityAuthority, CapabilityCall, CapabilityId, CapabilityRequest, CapabilityResult,
    CapabilityScope, ExecutionPermit, InvocationControl, MonoDeadline, MonoTime, OperationId,
    PermitId, PrincipalId, ProcessId, SandboxDecision, SandboxRequirement, ToolDefinition,
    UsageReport,
};
use kernel::capability::ToolExecutor;

fn descriptor(kind: ExtensionKind, name: &str, capability: &str) -> ExtensionDescriptor {
    let descriptor = ExtensionDescriptor::new(
        kind,
        name,
        "1.0.0",
        format!("{name} extension"),
        CapabilityId(capability.into()),
        RiskLevel::ReadOnly,
    )
    .unwrap();
    if matches!(kind, ExtensionKind::Tool | ExtensionKind::Mcp) {
        descriptor
            .with_tool_definition(ToolDefinition {
                name: capability.into(),
                description: format!("{name} tool"),
                input_schema: serde_json::json!({"type": "object"}),
            })
            .unwrap()
    } else {
        descriptor
    }
}

fn grant(capabilities: &[&str]) -> ExtensionGrant {
    ExtensionGrant {
        grant_id: "grant-1".into(),
        principal: PrincipalId("user:1".into()),
        session_id: "session-1".into(),
        agent_id: None,
        capabilities: capabilities
            .iter()
            .map(|name| CapabilityId((*name).into()))
            .collect(),
        resources: CapabilityScope {
            allowed_paths: vec!["/workspace".into()],
            allowed_targets: vec![],
            max_runtime_ms: Some(1_000),
            max_output_bytes: Some(4_096),
        },
    }
}

#[derive(Default)]
struct RecordingExecutor(AtomicUsize);

#[async_trait]
impl ToolExecutor for RecordingExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        CapabilityResult {
            call_id: request.call.call_id.clone(),
            output: "executed".into(),
            is_error: false,
            usage: UsageReport {
                permit_id: permit.id,
                exit_code: Some(0),
                ..Default::default()
            },
            audit_id: None,
            patch_delta: None,
        }
    }
}

fn service(executor: Arc<RecordingExecutor>) -> DefaultCorpusService {
    let catalog = ExtensionCatalog::new([
        descriptor(ExtensionKind::Tool, "read", "file.read"),
        descriptor(ExtensionKind::Skill, "review", "skill.review"),
        descriptor(ExtensionKind::Hook, "audit", "hook.audit"),
        descriptor(ExtensionKind::Plugin, "git", "plugin.git"),
        descriptor(ExtensionKind::Mcp, "remote-search", "mcp.search"),
    ])
    .unwrap();
    DefaultCorpusService::new(catalog, executor)
}

#[tokio::test]
async fn discovery_is_stable_and_never_eagerly_exposes_ungranted_extensions() {
    let service = service(Arc::new(RecordingExecutor::default()));

    let empty = service.catalog(&grant(&[])).await.unwrap();
    assert!(empty.entries.is_empty());

    let snapshot = service
        .catalog(&grant(&["file.read", "skill.review"]))
        .await
        .unwrap();
    let ids: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, vec!["skill:review", "tool:read"]);
    assert_eq!(
        ExtensionId::new(ExtensionKind::Tool, "read")
            .unwrap()
            .as_str(),
        "tool:read"
    );
}

#[tokio::test]
async fn activation_rejects_catalog_entries_outside_the_session_grant() {
    let service = service(Arc::new(RecordingExecutor::default()));
    let error = service
        .activate(ActivationRequest {
            grant: grant(&["file.read"]),
            extensions: vec![ExtensionId::new(ExtensionKind::Hook, "audit").unwrap()],
        })
        .await
        .unwrap_err();

    assert!(matches!(error, CorpusError::NotGranted(_)));
    assert_eq!(error.retry_disposition(), CorpusRetryDisposition::Never);
}

#[tokio::test]
async fn governed_invocation_requires_activation_binding_scope_and_permit() {
    let executor = Arc::new(RecordingExecutor::default());
    let service = service(executor.clone());
    let extension_id = ExtensionId::new(ExtensionKind::Tool, "read").unwrap();
    let activation = service
        .activate(ActivationRequest {
            grant: grant(&["file.read"]),
            extensions: vec![extension_id.clone()],
        })
        .await
        .unwrap();
    let operation_id = OperationId::new();
    let process_id = ProcessId::new();
    let scope = CapabilityScope {
        allowed_paths: vec!["/workspace".into()],
        allowed_targets: vec![],
        max_runtime_ms: Some(500),
        max_output_bytes: Some(2_048),
    };
    let request = CapabilityRequest {
        call: CapabilityCall {
            operation_id,
            process_id,
            name: "file.read".into(),
            input: serde_json::json!({"path": "/workspace/a"}),
            call_id: "call-1".into(),
            deadline: None,
        },
        authority: CapabilityAuthority {
            agent: None,
            principal: PrincipalId("user:1".into()),
            action: "read".into(),
            requested_scope: scope.clone(),
            risk: RiskLevel::ReadOnly,
            budget: None,
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
            connection_id: fabric::ConnectionId::new(),
            thread_id: fabric::ThreadId("session-1".into()),
            turn_id: fabric::TurnId::new(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/workspace".into(), vec![])
                .unwrap(),
            session_id: "session-1".into(),
            working_dir: "/workspace".into(),
        },
        control: InvocationControl::default(),
    };
    let permit = ExecutionPermit {
        id: PermitId::new(),
        operation_id,
        process_id,
        capability: CapabilityId("file.read".into()),
        granted_scope: scope,
        expires_at: MonoDeadline(MonoTime(10_000)),
        sandbox: SandboxDecision::NotApplicable,
        budget_reservation: None,
        lease: None,
    };

    let mut overbroad_request = request.clone();
    overbroad_request.authority.requested_scope = CapabilityScope::default();
    let error = service
        .invoke(GovernedInvocation {
            activation_id: activation.id,
            extension_id: extension_id.clone(),
            request: overbroad_request,
            permit: permit.clone(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, CorpusError::ScopeExceeded));
    assert_eq!(executor.0.load(Ordering::SeqCst), 0);

    let result = service
        .invoke(GovernedInvocation {
            activation_id: activation.id,
            extension_id,
            request,
            permit,
        })
        .await
        .unwrap();

    assert_eq!(result.output, "executed");
    assert_eq!(executor.0.load(Ordering::SeqCst), 1);
}

#[test]
fn catalog_rejects_duplicate_stable_ids() {
    let first = descriptor(ExtensionKind::Skill, "review", "skill.review");
    let second = descriptor(ExtensionKind::Skill, "review", "skill.other");
    let error = ExtensionCatalog::new([first, second]).unwrap_err();
    assert!(matches!(error, CorpusError::DuplicateExtension(_)));
}
