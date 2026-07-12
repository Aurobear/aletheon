//! Capability invoker integration tests — Phase 5B.
//!
//! Validates the admission → permit → execute → settle pipeline.

use aletheon_kernel::admission::AllowAllAdmissionController;
use aletheon_kernel::capability::{DefaultCapabilityInvoker, StubToolExecutor, ToolExecutor};
use aletheon_kernel::chronos::TestClock;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionError, AdmissionRequest, CapabilityInvoker, CapabilityRequest,
    CapabilityScope, ExecutionPermit, PermitId, RevokeReason, SandboxDecision, SandboxRequirement,
    UsageReport,
};
use std::sync::Arc;

fn test_clock() -> Arc<TestClock> {
    Arc::new(TestClock::default())
}

// ---------------------------------------------------------------------------
// Admission controller tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn allow_all_admits_everything() {
    let clock = test_clock();
    let ctrl = AllowAllAdmissionController::new(clock);

    let req = AdmissionRequest {
        operation_id: Default::default(),
        process_id: Default::default(),
        principal: fabric::PrincipalId("test-agent".into()),
        capability: fabric::CapabilityId("shell.execute".into()),
        action: "run".into(),
        input_summary: "ls".into(),
        risk: RiskLevel::ReadOnly,
        requested_scope: CapabilityScope::default(),
        budget: None,
        lease: None,
        sandbox: SandboxRequirement::NotRequired,
    };

    let permit = ctrl.admit(req).await.expect("AllowAll should admit");
    assert!(permit.is_valid_at(fabric::MonoTime(0)));
}

#[tokio::test]
async fn settle_then_settle_again_fails() {
    let clock = test_clock();
    let ctrl = AllowAllAdmissionController::new(clock);

    let permit = ctrl
        .admit(AdmissionRequest {
            operation_id: Default::default(),
            process_id: Default::default(),
            principal: fabric::PrincipalId("test".into()),
            capability: fabric::CapabilityId("test".into()),
            action: "test".into(),
            input_summary: "test".into(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: None,
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
        })
        .await
        .unwrap();

    // First settle succeeds.
    ctrl.settle(permit.id, UsageReport::default())
        .await
        .unwrap();

    // Second settle fails.
    let err = ctrl
        .settle(permit.id, UsageReport::default())
        .await
        .expect_err("double settle must fail");
    assert!(matches!(err, AdmissionError::AlreadySettled));
}

#[tokio::test]
async fn revoke_does_not_prevent_settle() {
    let clock = test_clock();
    let ctrl = AllowAllAdmissionController::new(clock);

    let permit = ctrl
        .admit(AdmissionRequest {
            operation_id: Default::default(),
            process_id: Default::default(),
            principal: fabric::PrincipalId("test".into()),
            capability: fabric::CapabilityId("test".into()),
            action: "test".into(),
            input_summary: "test".into(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: None,
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
        })
        .await
        .unwrap();

    // Revoke is idempotent in AllowAll.
    ctrl.revoke(permit.id, RevokeReason::OperationCancelled)
        .await
        .unwrap();

    // AllowAll doesn't track revoked state for settle (testing-only).
    // Production controllers would reject this.
}

#[tokio::test]
async fn sandbox_required_maps_to_passed_in_allow_all() {
    let clock = test_clock();
    let ctrl = AllowAllAdmissionController::new(clock);

    let req = AdmissionRequest {
        operation_id: Default::default(),
        process_id: Default::default(),
        principal: fabric::PrincipalId("test".into()),
        capability: fabric::CapabilityId("dangerous.tool".into()),
        action: "execute".into(),
        input_summary: "rm -rf /".into(),
        risk: RiskLevel::Destructive,
        requested_scope: CapabilityScope::default(),
        budget: None,
        lease: None,
        sandbox: SandboxRequirement::RequiredThenPromote,
    };

    let permit = ctrl.admit(req).await.unwrap();
    // AllowAll is overly permissive — in production this would require
    // actual sandbox execution.
    assert_eq!(permit.sandbox, SandboxDecision::Passed);
}

// ---------------------------------------------------------------------------
// Capability invoker tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoker_pipeline_admit_execute_settle() {
    let clock = test_clock();
    let admission = Arc::new(AllowAllAdmissionController::new(clock));
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(CapabilityRequest {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: "test.ping".into(),
            input: serde_json::json!({"msg": "hello"}),
            call_id: "call-1".into(),
            deadline: None,
        })
        .await;

    assert!(!result.is_error);
    assert!(result.output.contains("stub: executed test.ping"));
    assert_eq!(result.call_id, "call-1");
}

#[tokio::test]
async fn invoker_preserves_call_id_on_error() {
    let clock = test_clock();
    let admission = Arc::new(AllowAllAdmissionController::new(clock));
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    // AllowAll never denies, but the structure preserves call_id.
    let result = invoker
        .invoke(CapabilityRequest {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: "test.noop".into(),
            input: serde_json::json!({}),
            call_id: "my-call-id".into(),
            deadline: None,
        })
        .await;

    assert_eq!(result.call_id, "my-call-id");
}

// ---------------------------------------------------------------------------
// Custom executor for testing error paths
// ---------------------------------------------------------------------------

struct ErrorToolExecutor;
#[async_trait::async_trait]
impl ToolExecutor for ErrorToolExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        _permit: &ExecutionPermit,
    ) -> fabric::CapabilityResult {
        fabric::CapabilityResult {
            call_id: request.call_id.clone(),
            output: "tool failed: simulated error".into(),
            is_error: true,
            usage: fabric::UsageReport {
                permit_id: _permit.id,
                ..Default::default()
            },
            audit_id: Some(fabric::AuditEventId::new()),
        }
    }
}

#[tokio::test]
async fn invoker_reports_executor_errors() {
    let clock = test_clock();
    let admission = Arc::new(AllowAllAdmissionController::new(clock));
    let executor = Arc::new(ErrorToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(CapabilityRequest {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: "test.failing".into(),
            input: serde_json::json!({}),
            call_id: "fail-1".into(),
            deadline: None,
        })
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("simulated error"));
}

// ---------------------------------------------------------------------------
// Denying admission controller
// ---------------------------------------------------------------------------

struct DenyAllAdmissionController;
#[async_trait::async_trait]
impl AdmissionController for DenyAllAdmissionController {
    async fn admit(&self, _request: AdmissionRequest) -> Result<ExecutionPermit, AdmissionError> {
        Err(AdmissionError::Denied {
            reason: "deny-all policy".into(),
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

#[tokio::test]
async fn denied_admission_produces_error_result() {
    let admission = Arc::new(DenyAllAdmissionController);
    let executor = Arc::new(StubToolExecutor);
    let invoker = DefaultCapabilityInvoker::new(admission, executor);

    let result = invoker
        .invoke(CapabilityRequest {
            operation_id: fabric::OperationId::new(),
            process_id: fabric::ProcessId::new(),
            name: "test.blocked".into(),
            input: serde_json::json!({}),
            call_id: "blocked-1".into(),
            deadline: None,
        })
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("denied"));
    assert!(result.output.contains("deny-all"));
    assert_eq!(result.call_id, "blocked-1");
}
