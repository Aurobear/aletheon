use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use async_trait::async_trait;
use executive::service::governed_capability::{
    AuthorizedInvocation, GovernedCapabilityInvoker, RegistryAuthorityProvider,
    TurnAuthorityProvider, TurnCapabilityInvoker,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityRequest, CapabilityResult,
    CapabilityScope, ConsciousArbitrationMode, InvocationControl, PrincipalId, SandboxRequirement,
    ToolEventSink, ToolProgress, ToolResult, ToolResultMeta, UsageReport,
};

struct RecordingAuthority {
    events: Arc<Mutex<Vec<&'static str>>>,
    reject: bool,
}

#[async_trait]
impl TurnAuthorityProvider for RecordingAuthority {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        self.events.lock().unwrap().push("authorize");
        if self.reject {
            bail!("policy rejected");
        }
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                agent: None,
                principal: PrincipalId("trusted-application".into()),
                action: call.name.clone(),
                requested_scope: CapabilityScope::default(),
                risk: RiskLevel::SystemModify,
                budget: None,
                lease: None,
                sandbox: SandboxRequirement::Required,
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId("session-1".into()),
                turn_id: fabric::TurnId::new(),
                workspace: fabric::WorkspacePolicy::from_resolved_roots(
                    "/trusted/workspace".into(),
                    vec![],
                )
                .unwrap(),
                session_id: "session-1".into(),
                working_dir: "/trusted/workspace".into(),
            },
            control: InvocationControl::default(),
        })
    }
}

struct RecordingInner {
    events: Arc<Mutex<Vec<&'static str>>>,
    requests: Arc<Mutex<Vec<CapabilityRequest>>>,
}

struct StreamingInner {
    events: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl CapabilityInvoker for StreamingInner {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        self.events.lock().unwrap().push("legacy");
        CapabilityResult {
            call_id: request.call.call_id,
            output: "settled".into(),
            is_error: false,
            usage: UsageReport::default(),
            audit_id: Some(fabric::AuditEventId::new()),
        }
    }

    async fn invoke_streaming(
        &self,
        request: CapabilityRequest,
        sink: &mut ToolEventSink,
    ) -> CapabilityResult {
        self.events.lock().unwrap().push("streaming");
        assert!(sink.progress(ToolProgress::Text("working".into())));
        let result = CapabilityResult {
            call_id: request.call.call_id,
            output: "settled".into(),
            is_error: false,
            usage: UsageReport::default(),
            audit_id: Some(fabric::AuditEventId::new()),
        };
        sink.terminal(Ok(ToolResult {
            content: result.output.clone(),
            is_error: result.is_error,
            metadata: ToolResultMeta::default(),
        }))
        .await;
        result
    }
}

#[async_trait]
impl CapabilityInvoker for RecordingInner {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        self.events.lock().unwrap().push("inner");
        self.requests.lock().unwrap().push(request.clone());
        CapabilityResult {
            call_id: request.call.call_id,
            output: "ok".into(),
            is_error: false,
            usage: UsageReport::default(),
            audit_id: None,
        }
    }
}

fn call() -> CapabilityCall {
    CapabilityCall {
        operation_id: fabric::OperationId::new(),
        process_id: fabric::ProcessId::new(),
        name: "file_write".into(),
        input: serde_json::json!({"path":"x"}),
        call_id: "call-1".into(),
        deadline: None,
    }
}

#[tokio::test]
async fn authorization_precedes_inner_and_attaches_trusted_policy() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(RecordingInner {
            events: events.clone(),
            requests: requests.clone(),
        }),
        Arc::new(RecordingAuthority {
            events: events.clone(),
            reject: false,
        }),
    )
    .with_arbitration_mode(ConsciousArbitrationMode::Enforce);

    let result = invoker.invoke(call()).await;
    assert!(!result.is_error);
    assert_eq!(*events.lock().unwrap(), ["authorize", "inner"]);
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].authority.principal.0, "trusted-application");
    assert_eq!(requests[0].authority.risk, RiskLevel::SystemModify);
    assert_eq!(requests[0].authority.sandbox, SandboxRequirement::Required);
}

#[tokio::test]
async fn authorization_rejection_never_reaches_inner() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(RecordingInner {
            events: events.clone(),
            requests: requests.clone(),
        }),
        Arc::new(RecordingAuthority {
            events: events.clone(),
            reject: true,
        }),
    );

    let result = invoker.invoke(call()).await;
    assert!(result.is_error);
    assert!(result.output.contains("policy rejected"));
    assert_eq!(*events.lock().unwrap(), ["authorize"]);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn streaming_flag_path_forwards_progress_and_preserves_settled_result() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (mut stream, sender) =
        fabric::ipc::TurnEventStream::new(fabric::ipc::StreamConfig::turn_events(8));
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(StreamingInner {
            events: events.clone(),
        }),
        Arc::new(RecordingAuthority {
            events: Arc::new(Mutex::new(Vec::new())),
            reject: false,
        }),
    )
    .with_tool_stream(sender);

    let result = invoker.invoke(call()).await;

    assert!(!result.is_error);
    assert_eq!(result.output, "settled");
    assert!(result.audit_id.is_some(), "settlement audit must survive");
    assert_eq!(*events.lock().unwrap(), ["streaming"]);
    assert!(matches!(
        stream.try_recv(),
        Some(Ok(fabric::ipc::TurnEventV1::ToolProgress {
            name,
            call_id,
            kind,
            payload,
        })) if name == "file_write"
            && call_id == "call-1"
            && kind == "text"
            && payload == serde_json::json!("working")
    ));
}

#[tokio::test]
async fn model_arguments_cannot_replace_capability_workspace() {
    let workspace = fabric::WorkspacePolicy::from_resolved_roots(
        "/trusted/workspace".into(),
        vec!["/trusted/shared".into()],
    )
    .unwrap();
    let provider = RegistryAuthorityProvider::new(
        std::collections::HashMap::from([("file_write".to_string(), RiskLevel::SystemModify)]),
        PrincipalId("trusted-application".into()),
        fabric::ConnectionId::new(),
        fabric::ThreadId("thread-1".into()),
        fabric::TurnId::new(),
        workspace.clone(),
        "session-1".into(),
        "/etc".into(),
        SandboxRequirement::Required,
        tokio_util::sync::CancellationToken::new(),
    );

    let authorized = provider.authorize(&call()).await.unwrap();
    assert_eq!(authorized.authority.workspace, workspace);
    assert_eq!(
        authorized.authority.working_dir,
        std::path::PathBuf::from("/trusted/workspace")
    );
}
