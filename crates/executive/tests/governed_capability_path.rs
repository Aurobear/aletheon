use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use async_trait::async_trait;
use executive::service::governed_capability::{
    AuthorizedInvocation, GovernedCapabilityInvoker, TurnAuthorityProvider, TurnCapabilityInvoker,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityRequest, CapabilityResult,
    CapabilityScope, InvocationControl, PrincipalId, SandboxRequirement, UsageReport,
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
                principal: PrincipalId("trusted-application".into()),
                action: call.name.clone(),
                requested_scope: CapabilityScope::default(),
                risk: RiskLevel::SystemModify,
                budget: None,
                lease: None,
                sandbox: SandboxRequirement::Required,
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
    );

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
