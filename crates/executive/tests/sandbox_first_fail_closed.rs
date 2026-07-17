//! SandboxFirst fail-closed regression tests — M0-PR-0E.
//!
//! When `SandboxFirst` verdict is triggered, every capability invocation must
//! pass through the admission controller with `SandboxRequirement::Required`.
//! If sandbox infrastructure is unavailable, `SandboxDecision::Required` is
//! returned and execution MUST be denied — fail closed, never open.
//!
//! This test file is mandated by `docs/arch/07_M0_DETAILED_IMPLEMENTATION_PLAN.md`
//! and implements the acceptance criteria in `docs/arch/06_PR_PLAN_AND_ACCEPTANCE.md`.

use aletheon_kernel::admission::AllowAllAdmissionController;
use aletheon_kernel::capability::{DefaultCapabilityInvoker, StubToolExecutor};
use fabric::{
    AdmissionController, AdmissionError, AdmissionRequest, CapabilityAuthority, CapabilityCall,
    CapabilityInvoker, CapabilityRequest, CapabilityScope, ExecutionPermit, InvocationControl,
    PermitId, PrincipalId, RevokeReason, SandboxDecision, SandboxRequirement, UsageReport,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SandboxRequired stub — admission controller that returns Required
// ---------------------------------------------------------------------------

/// An admission controller that always grants a permit, but with
/// `SandboxDecision::Required`. This simulates the path triggered when
/// `SelfField` returns `Verdict::SandboxFirst` and the daemon has no sandbox
/// infrastructure: the permit is granted but `SandboxDecision::Required`
/// signals that execution must be blocked at the capability-invoker layer.
struct SandboxRequiredAdmissionController;

#[async_trait::async_trait]
impl AdmissionController for SandboxRequiredAdmissionController {
    async fn admit(&self, request: AdmissionRequest) -> Result<ExecutionPermit, AdmissionError> {
        Ok(ExecutionPermit {
            id: PermitId::new(),
            operation_id: request.operation_id,
            process_id: request.process_id,
            capability: request.capability,
            granted_scope: CapabilityScope::default(),
            expires_at: fabric::MonoDeadline::after(fabric::MonoTime(0), 60_000),
            sandbox: SandboxDecision::Required,
            budget_reservation: None,
            lease: None,
        })
    }

    async fn settle(
        &self,
        _permit_id: PermitId,
        _usage: UsageReport,
    ) -> Result<(), AdmissionError> {
        Ok(())
    }

    async fn revoke(
        &self,
        _permit_id: PermitId,
        _reason: RevokeReason,
    ) -> Result<(), AdmissionError> {
        Ok(())
    }
}

fn request(
    name: &str,
    input: serde_json::Value,
    call_id: &str,
    sandbox: SandboxRequirement,
) -> CapabilityRequest {
    CapabilityRequest {
        call: CapabilityCall {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: name.into(),
            input,
            call_id: call_id.into(),
            deadline: None,
        },
        authority: CapabilityAuthority {
            agent: None,
            principal: PrincipalId("test".into()),
            action: name.into(),
            requested_scope: CapabilityScope::default(),
            risk: if sandbox == SandboxRequirement::NotRequired {
                fabric::types::admission::RiskLevel::ReadOnly
            } else {
                fabric::types::admission::RiskLevel::Destructive
            },
            budget: None,
            lease: None,
            sandbox,
            connection_id: fabric::ConnectionId::new(),
            thread_id: fabric::ThreadId("test".into()),
            turn_id: fabric::TurnId::new(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots(std::env::temp_dir(), vec![])
                .unwrap(),
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
        },
        control: InvocationControl::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sandbox_first_required_fail_closed_via_invoker() {
    // When SelfField returns SandboxFirst, the daemon sets
    // SandboxRequirement::Required on every tool call.  If sandbox
    // infrastructure is absent, the admission controller returns
    // SandboxDecision::Required, and the capability invoker MUST
    // fail closed — the tool MUST NOT execute.
    let admission = Arc::new(SandboxRequiredAdmissionController);
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(request(
            "dangerous.tool",
            serde_json::json!({"action": "rm -rf /"}),
            "sandbox-required-1",
            SandboxRequirement::Required,
        ))
        .await;

    // Fail-closed: must return error, not execute.
    assert!(
        result.is_error,
        "SandboxDecision::Required must fail closed"
    );
    assert!(
        result.output.to_lowercase().contains("sandbox"),
        "Error must mention sandbox: {}",
        result.output
    );
    assert!(
        result.output.contains("not available"),
        "Error must explain sandbox unavailable: {}",
        result.output
    );
    assert_eq!(result.call_id, "sandbox-required-1");
}

#[tokio::test]
async fn sandbox_first_request_with_required_sandbox_fails_closed() {
    // Full admission → invoker pipeline: when the request carries
    // SandboxRequirement::Required (as the daemon sets after SandboxFirst),
    // and the admission controller reflects that as SandboxDecision::Required,
    // execution must be denied.
    let admission = Arc::new(SandboxRequiredAdmissionController);
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(request(
            "shell.execute",
            serde_json::json!({"cmd": "whoami"}),
            "sf-1",
            SandboxRequirement::Required,
        ))
        .await;

    assert!(result.is_error);
    assert!(
        result.output.contains("Sandbox required"),
        "Expected sandbox fail-closed message, got: {}",
        result.output
    );
}

#[tokio::test]
async fn allow_all_still_passes_without_sandbox() {
    // Regression: AllowAllAdmissionController (used for testing and
    // non-dangerous paths) must allow execution when sandbox is not required.
    let admission = Arc::new(AllowAllAdmissionController::new(Arc::new(
        aletheon_kernel::chronos::TestClock::default(),
    )));
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(request(
            "safe.readonly",
            serde_json::json!({"query": "hello"}),
            "safe-1",
            SandboxRequirement::NotRequired,
        ))
        .await;

    assert!(!result.is_error, "AllowAll should permit safe operations");
    assert!(
        result.output.contains("stub: executed"),
        "Safe tool should execute: {}",
        result.output
    );
}

#[tokio::test]
async fn sandbox_decision_required_makes_permit_valid_at_false() {
    // Contract: SandboxDecision::Required means the permit itself reports
    // as invalid (is_valid_at returns false), providing defense in depth.
    // Even if a caller bypasses the capability invoker check, the permit
    // signals invalidity.
    let permit = ExecutionPermit {
        id: PermitId::new(),
        operation_id: Default::default(),
        process_id: Default::default(),
        capability: fabric::CapabilityId("test".into()),
        granted_scope: CapabilityScope::default(),
        expires_at: fabric::MonoDeadline::after(fabric::MonoTime(0), 60_000),
        sandbox: SandboxDecision::Required,
        budget_reservation: None,
        lease: None,
    };

    assert!(
        !permit.is_valid_at(fabric::MonoTime(0)),
        "SandboxDecision::Required must invalidate the permit (defense in depth)"
    );
}

#[tokio::test]
async fn sandbox_required_on_destructive_request_fails_closed() {
    // Highest-risk path: Destructive action + SandboxRequired + no sandbox
    // infrastructure. Must fail closed under all conditions.
    let admission = Arc::new(SandboxRequiredAdmissionController);
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(request(
            "filesystem.delete_all",
            serde_json::json!({"path": "/important/data"}),
            "destructive-1",
            SandboxRequirement::Required,
        ))
        .await;

    assert!(
        result.is_error,
        "Destructive + SandboxRequired must fail closed"
    );
    assert!(
        result.output.contains("Sandbox required"),
        "Error must indicate sandbox: {}",
        result.output
    );
}
